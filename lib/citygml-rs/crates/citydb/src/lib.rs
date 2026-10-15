//! A 3DCityDB v5 database read as CityGML model objects.
//!
//! [`CityDb::connect`] opens a database ([`Config`]: the connection, as
//! libpq takes it, and the schema) inside one read-only transaction, so
//! that every read sees the same snapshot. [`CityDb::objects`] then
//! streams its top-level city objects, a batch of features at a time, as
//! version-neutral [`citygml_core::GmlObject`]s, the model the CityGML 2.0
//! and 3.0 readers produce; with the `cjseq` feature,
//! [`CityDb::features`] turns them into `cjseq::CityJSONFeature`s.
//!
//! A database stores the CityGML 3.0 model generically: every feature is a
//! row, every property a row of a property tree, every geometry PostGIS
//! with a JSON description of its GML structure. The reader rebuilds each
//! object as the CityGML 3.0 reader reads the same object from XML.

mod appearance;
mod assemble;
mod catalogue;
mod config;
mod db;
mod geometry;
mod rows;
mod stream;
mod tls;
mod wkb;

use std::sync::{Arc, Mutex};

use citygml_core::{Error, Result};

pub use config::{Config, SslMode, has_password};
pub use stream::Objects;

use catalogue::Catalogue;
use db::Session;
use stream::Prepared;

/// Whether `id` is one citydb-tool generated on import for an object the
/// source gave none (`ID_` and a UUID). Such an id names nothing the
/// source named, so a material keeps no name from it.
pub(crate) fn is_generated_id(id: &str) -> bool {
    id.strip_prefix("ID_").is_some_and(|uuid| {
        uuid.len() == 36
            && uuid.char_indices().all(|(i, c)| match i {
                8 | 13 | 18 | 23 => c == '-',
                _ => c.is_ascii_hexdigit(),
            })
    })
}

/// The reference system a database stores its coordinates in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Srs {
    /// The SRID PostGIS knows it by: an EPSG code, or 0 for none.
    pub srid: i32,
    /// The name the database gives it, such as
    /// `urn:ogc:def:crs,crs:EPSG::28992,crs:EPSG::5109`.
    pub name: String,
}

/// An open 3DCityDB v5 database: one connection, inside one read-only
/// transaction for as long as it is open.
pub struct CityDb {
    session: Mutex<Session>,
    catalogue: Catalogue,
    srs: Srs,
    version: String,
    name: String,
    batch_features: usize,
    /// What every pass shares, read on the first.
    prepared: Mutex<Option<Arc<Prepared>>>,
}

impl CityDb {
    /// Connect as `config` says, and check that its schema holds a
    /// 3DCityDB v5 instance.
    pub fn connect(config: &Config) -> Result<Self> {
        let resolved = config::resolve(&config.connection, &config::ProcessEnvironment)?;
        let client = tls::connect(&resolved)?;
        let mut session = Session::begin(client, &config.schema)?;
        let schema = &config.schema;
        let exists = session
            .query_opt("SELECT 1 FROM pg_namespace WHERE nspname = $1", &[schema])?
            .is_some();
        if !exists {
            return Err(Error::invalid(format!(
                "the database has no schema {schema:?}"
            )));
        }
        let tables = session
            .query_opt(
                "SELECT to_regclass('{s}.feature') IS NOT NULL, \
                        to_regclass('{s}.cityobject') IS NOT NULL",
                &[],
            )?
            .map(|row| (row.get::<_, bool>(0), row.get::<_, bool>(1)));
        match tables {
            Some((true, _)) => {}
            Some((false, true)) => {
                return Err(Error::invalid(format!(
                    "schema {schema:?} is a 3DCityDB v4 instance; only v5 is read"
                )));
            }
            _ => {
                return Err(Error::invalid(format!(
                    "schema {schema:?} holds no 3DCityDB instance"
                )));
            }
        }
        let version = session
            .query_opt(
                "SELECT version, major_version FROM citydb_pkg.citydb_version()",
                &[],
            )
            .map_err(|e| Error::invalid(format!("not a 3DCityDB database: {e}")))?
            .map(|row| (row.get::<_, String>(0), row.get::<_, i32>(1)));
        let version = match version {
            Some((version, 5)) => version,
            Some((version, _)) => {
                return Err(Error::invalid(format!(
                    "the database is 3DCityDB {version}; only v5 is read"
                )));
            }
            None => return Err(Error::invalid("not a 3DCityDB database")),
        };
        let srs = session
            .query_opt("SELECT srid, srs_name FROM {s}.database_srs", &[])?
            .map(|row| Srs {
                srid: row.get(0),
                name: row.get::<_, Option<String>>(1).unwrap_or_default(),
            })
            .ok_or_else(|| {
                Error::invalid(format!("schema {schema:?} states no reference system"))
            })?;
        let catalogue = Catalogue::load(&mut session)?;
        Ok(Self {
            session: Mutex::new(session),
            catalogue,
            srs,
            version,
            name: format!("{}, schema {schema:?}", tls::describe(&resolved)),
            batch_features: config.batch_features.max(1),
            prepared: Mutex::new(None),
        })
    }

    /// The reference system the database stores its coordinates in.
    pub fn srs(&self) -> &Srs {
        &self.srs
    }

    /// The 3DCityDB version of the instance (`5.1.2`).
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The database, its host and user, and the schema, for messages: never
    /// the password.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What every pass over the database shares, as the CityGML readers'
    /// document pass gives it: the geometry templates the database's
    /// implicit geometries place, and which groups reference which members.
    pub fn document(&self) -> Result<Arc<citygml_core::reader::Document>> {
        Ok(self.prepared()?.document.clone())
    }

    /// The 3D extent of the top-level features the reader yields, `[minx,
    /// miny, minz, maxx, maxy, maxz]`, or `None` when there are none.
    pub fn extent(&self) -> Result<Option<[f64; 6]>> {
        let sql = format!(
            "SELECT ST_XMin(e), ST_YMin(e), ST_ZMin(e), ST_XMax(e), ST_YMax(e), ST_ZMax(e) \
             FROM (SELECT ST_3DExtent(f.envelope) AS e FROM {{s}}.feature f WHERE {}) x",
            stream::ROOTS
        );
        let row = self.session()?.query_opt(&sql, &[])?;
        Ok(row.and_then(|row| {
            let values: Vec<Option<f64>> = (0..6).map(|i| row.get(i)).collect();
            let values: Option<Vec<f64>> = values.into_iter().collect();
            values.map(|v| [v[0], v[1], v[2], v[3], v[4], v[5]])
        }))
    }

    /// The database's top-level city objects, in feature id order, each
    /// with what it contains. Each call is a new pass over the same
    /// snapshot.
    pub fn objects(&self) -> Result<Objects<'_>> {
        Ok(Objects::new(self, self.prepared()?))
    }

    /// What the passes share: the templates and the group index, read once.
    fn prepared(&self) -> Result<Arc<Prepared>> {
        let mut cached = self
            .prepared
            .lock()
            .map_err(|_| Error::invalid("the database reader was poisoned by a panic"))?;
        if let Some(prepared) = cached.as_ref() {
            return Ok(prepared.clone());
        }
        let prepared = Arc::new(stream::prepare(&mut *self.session()?)?);
        *cached = Some(prepared.clone());
        Ok(prepared)
    }

    fn session(&self) -> Result<std::sync::MutexGuard<'_, Session>> {
        self.session
            .lock()
            .map_err(|_| Error::invalid("the database connection was poisoned by a panic"))
    }
}

#[cfg(feature = "cjseq")]
impl CityDb {
    /// The objects as `cjseq::CityJSONFeature`s, each with its own vertex
    /// pool quantised against `transform`.
    pub fn features(&self, transform: cjseq::Transform) -> Result<Features<'_>> {
        Ok(Features::new(self.objects()?, transform))
    }
}

/// See [`CityDb::features`].
#[cfg(feature = "cjseq")]
pub type Features<'a> = citygml_core::reader::Features<Objects<'a>>;
