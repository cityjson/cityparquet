//! Where the database is and how to reach it, resolved as libpq resolves a
//! connection: what the URI states first, then the `PG*` environment
//! variables, then libpq's defaults, and a password from `~/.pgpass` when
//! nothing else gives one.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use citygml_core::{Error, Result};

/// How to reach a 3DCityDB v5 database and read it.
#[derive(Debug, Clone)]
pub struct Config {
    /// A `postgresql://` (or `postgres://`) URI, as libpq takes it:
    /// `postgresql://alice@db.example.org:5432/citydb?sslmode=require`.
    /// What it leaves out comes from `PGHOST`, `PGPORT`, `PGUSER`,
    /// `PGPASSWORD`, `PGDATABASE`, `PGSSLMODE` and `PGSSLROOTCERT`, and a
    /// password from the file `PGPASSFILE` names, else `~/.pgpass`.
    pub connection: String,
    /// The schema the 3DCityDB instance is in.
    pub schema: String,
    /// How many top-level features are read from the database at a time.
    pub batch_features: usize,
}

impl Config {
    /// A connection to `connection`, schema `citydb`, a thousand top-level
    /// features at a time.
    pub fn new(connection: impl Into<String>) -> Self {
        Self {
            connection: connection.into(),
            schema: "citydb".into(),
            batch_features: 1000,
        }
    }
}

/// Whether, and how, the connection is encrypted: libpq's `sslmode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SslMode {
    /// No TLS.
    Disable,
    /// TLS when the server offers it, the certificate not verified.
    Prefer,
    /// TLS, the certificate not verified.
    Require,
    /// TLS, the certificate verified against the trusted roots and the host
    /// name. libpq's `verify-ca` is read as this, stricter, mode.
    VerifyFull,
}

impl FromStr for SslMode {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "disable" => Self::Disable,
            "allow" | "prefer" => Self::Prefer,
            "require" => Self::Require,
            "verify-ca" | "verify-full" => Self::VerifyFull,
            other => {
                return Err(Error::invalid(format!(
                    "sslmode {other:?} is not one of disable, allow, prefer, require, verify-ca, verify-full"
                )));
            }
        })
    }
}

/// A connection with everything libpq would fill in filled in.
pub(crate) struct Resolved {
    pub(crate) postgres: postgres::Config,
    pub(crate) ssl_mode: SslMode,
    /// Extra trusted roots, PEM (`sslrootcert`).
    pub(crate) root_cert: Option<PathBuf>,
}

/// What the process environment says, so tests can give their own.
pub(crate) trait Environment {
    fn var(&self, name: &str) -> Option<String>;
    fn home(&self) -> Option<PathBuf>;
}

pub(crate) struct ProcessEnvironment;

impl Environment for ProcessEnvironment {
    fn var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|v| !v.is_empty())
    }

    fn home(&self) -> Option<PathBuf> {
        self.var("HOME").map(PathBuf::from)
    }
}

/// Resolve `uri` as libpq would.
pub(crate) fn resolve(uri: &str, env: &dyn Environment) -> Result<Resolved> {
    if !(uri.starts_with("postgresql://") || uri.starts_with("postgres://")) {
        return Err(Error::invalid(format!(
            "{uri:?} is not a postgresql:// connection URI"
        )));
    }
    // rust-postgres knows only some of libpq's parameters; the TLS ones are
    // taken out here and handled by this crate.
    let (base, query) = match uri.split_once('?') {
        Some((base, query)) => (base, Some(query)),
        None => (uri, None),
    };
    let mut kept = Vec::new();
    let mut ssl_mode = None;
    let mut root_cert = None;
    for pair in query.into_iter().flat_map(|q| q.split('&')) {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "sslmode" => ssl_mode = Some(value.parse::<SslMode>()?),
            "sslrootcert" => root_cert = Some(PathBuf::from(percent_decode(value))),
            _ => kept.push(pair),
        }
    }
    let uri = if kept.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", kept.join("&"))
    };
    let mut config = postgres::Config::from_str(&uri)
        .map_err(|e| Error::invalid(format!("invalid connection URI: {e}")))?;

    if config.get_hosts().is_empty() {
        let hosts = env.var("PGHOST").unwrap_or_else(|| "localhost".into());
        for host in hosts.split(',') {
            config.host(host);
        }
    }
    if config.get_ports().is_empty() {
        let port = match env.var("PGPORT") {
            Some(p) => p
                .parse::<u16>()
                .map_err(|_| Error::invalid(format!("PGPORT {p:?} is not a port number")))?,
            None => 5432,
        };
        config.port(port);
    }
    if config.get_user().is_none() {
        let user = env
            .var("PGUSER")
            .or_else(|| env.var("USER"))
            .ok_or_else(|| Error::invalid("the connection names no user, and PGUSER is not set"))?;
        config.user(&user);
    }
    if config.get_dbname().is_none() {
        let dbname = env
            .var("PGDATABASE")
            .or_else(|| config.get_user().map(str::to_string))
            .unwrap_or_default();
        config.dbname(&dbname);
    }
    if config.get_password().is_none() {
        let password = env.var("PGPASSWORD").or_else(|| {
            let file = env
                .var("PGPASSFILE")
                .map(PathBuf::from)
                .or_else(|| env.home().map(|h| h.join(".pgpass")))?;
            pgpass(&file, &config)
        });
        if let Some(password) = password {
            config.password(password);
        }
    }
    let ssl_mode = match ssl_mode {
        Some(mode) => mode,
        None => match env.var("PGSSLMODE") {
            Some(mode) => mode.parse()?,
            None => SslMode::Prefer,
        },
    };
    let root_cert = root_cert.or_else(|| env.var("PGSSLROOTCERT").map(PathBuf::from));
    config.ssl_mode(match ssl_mode {
        SslMode::Disable => postgres::config::SslMode::Disable,
        SslMode::Prefer => postgres::config::SslMode::Prefer,
        SslMode::Require | SslMode::VerifyFull => postgres::config::SslMode::Require,
    });
    Ok(Resolved {
        postgres: config,
        ssl_mode,
        root_cert,
    })
}

/// The password `~/.pgpass` (or the file at `path`) gives the connection:
/// the first line whose `host:port:database:user` matches, `*` matching
/// anything, `\:` and `\\` escaping. A missing or unreadable file gives
/// none.
fn pgpass(path: &Path, config: &postgres::Config) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let host = match config.get_hosts().first()? {
        postgres::config::Host::Tcp(h) => h.clone(),
        #[cfg(unix)]
        postgres::config::Host::Unix(_) => "localhost".to_string(),
    };
    let port = config
        .get_ports()
        .first()
        .copied()
        .unwrap_or(5432)
        .to_string();
    let wanted = [
        host.as_str(),
        port.as_str(),
        config.get_dbname().unwrap_or_default(),
        config.get_user().unwrap_or_default(),
    ];
    text.lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .find_map(|line| {
            let fields = pgpass_fields(line);
            if fields.len() != 5 {
                return None;
            }
            let matches = fields[..4]
                .iter()
                .zip(wanted)
                .all(|(field, want)| field == "*" || field == want);
            matches.then(|| fields[4].clone())
        })
}

/// A `.pgpass` line's colon-separated fields, unescaped.
fn pgpass_fields(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    fields.last_mut().expect("non-empty").push(next);
                }
            }
            ':' => fields.push(String::new()),
            c => fields.last_mut().expect("non-empty").push(c),
        }
    }
    fields
}

/// `%XX` escapes decoded, as a URI query value spells them.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push((high * 16 + low) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether the URI carries a password, which a command line shows to every
/// user of the machine.
pub fn has_password(uri: &str) -> bool {
    let rest = uri
        .strip_prefix("postgresql://")
        .or_else(|| uri.strip_prefix("postgres://"))
        .unwrap_or(uri);
    let authority = rest.split(['/', '?']).next().unwrap_or_default();
    authority
        .rsplit_once('@')
        .is_some_and(|(userinfo, _)| userinfo.contains(':'))
        || uri.contains("password=")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Fixed(HashMap<&'static str, String>, Option<PathBuf>);

    impl Environment for Fixed {
        fn var(&self, name: &str) -> Option<String> {
            self.0.get(name).cloned()
        }
        fn home(&self) -> Option<PathBuf> {
            self.1.clone()
        }
    }

    fn env(vars: &[(&'static str, &str)]) -> Fixed {
        Fixed(
            vars.iter().map(|(k, v)| (*k, (*v).to_string())).collect(),
            None,
        )
    }

    #[test]
    fn what_the_uri_leaves_out_comes_from_the_environment() {
        let env = env(&[
            ("PGHOST", "db.example.org"),
            ("PGPORT", "6543"),
            ("PGUSER", "alice"),
            ("PGDATABASE", "citydb"),
            ("PGPASSWORD", "secret"),
        ]);
        let r = resolve("postgresql://", &env).unwrap();
        let c = &r.postgres;
        assert!(
            matches!(&c.get_hosts()[0], postgres::config::Host::Tcp(h) if h == "db.example.org")
        );
        assert_eq!(c.get_ports(), &[6543]);
        assert_eq!(c.get_user(), Some("alice"));
        assert_eq!(c.get_dbname(), Some("citydb"));
        assert_eq!(c.get_password(), Some(&b"secret"[..]));
        assert_eq!(r.ssl_mode, SslMode::Prefer);
    }

    #[test]
    fn the_uri_wins_over_the_environment() {
        let env = env(&[("PGHOST", "elsewhere"), ("PGUSER", "bob")]);
        let r = resolve(
            "postgresql://alice@db.example.org:5433/delft?sslmode=verify-full",
            &env,
        )
        .unwrap();
        assert!(
            matches!(&r.postgres.get_hosts()[0], postgres::config::Host::Tcp(h) if h == "db.example.org")
        );
        assert_eq!(r.postgres.get_user(), Some("alice"));
        assert_eq!(r.postgres.get_dbname(), Some("delft"));
        assert_eq!(r.ssl_mode, SslMode::VerifyFull);
    }

    #[test]
    fn a_password_comes_from_the_password_file() {
        let dir = std::env::temp_dir().join(format!("citydb-pgpass-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("pgpass");
        std::fs::write(
            &file,
            "# comment\nother:5432:*:alice:wrong\ndb.example.org:*:*:alice:pa\\:ss\n",
        )
        .unwrap();
        let env = env(&[("PGPASSFILE", file.to_str().unwrap())]);
        let r = resolve("postgresql://alice@db.example.org/citydb", &env).unwrap();
        assert_eq!(r.postgres.get_password(), Some(&b"pa:ss"[..]));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_unknown_sslmode_is_refused() {
        assert!(resolve("postgresql://a@h/d?sslmode=sometimes", &env(&[])).is_err());
    }

    #[test]
    fn a_password_in_the_uri_is_noticed() {
        assert!(has_password("postgresql://alice:secret@host/db"));
        assert!(has_password("postgresql://host/db?password=secret"));
        assert!(!has_password("postgresql://alice@host/db"));
        assert!(!has_password("postgresql://host/db?sslmode=require"));
    }
}
