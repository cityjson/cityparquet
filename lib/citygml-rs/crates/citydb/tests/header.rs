//! What a database says about itself: its version, reference system and
//! extent.

mod common;

#[test]
fn reads_the_reference_system_of_the_database() {
    let Some(db) = common::database("delft") else {
        return;
    };
    assert_eq!(db.srs().srid, 7415);
    assert_eq!(
        db.srs().name,
        "urn:ogc:def:crs,crs:EPSG::28992,crs:EPSG::5109"
    );
    let Some(db) = common::database("plateau_bldg") else {
        return;
    };
    assert_eq!(db.srs().srid, 6697);
}

#[test]
fn the_extent_covers_every_top_level_feature() {
    let Some(db) = common::database("delft") else {
        return;
    };
    // The 3DBAG tile states its extent in its header; the database's is that
    // of the 100 buildings imported from it, so lies inside it, up to the
    // millimetre the tile's vertices are quantised to.
    let tile = [
        84501.5546875,
        445805.03125,
        -3.746997833251953,
        85675.234375,
        446983.46875,
        95.04200744628906,
    ];
    let extent = db
        .extent()
        .unwrap()
        .expect("a database with features has an extent");
    let step = 0.001;
    for axis in 0..3 {
        assert!(extent[axis] >= tile[axis] - step, "{extent:?}");
        assert!(extent[axis] < extent[axis + 3], "{extent:?}");
        assert!(extent[axis + 3] <= tile[axis + 3] + step, "{extent:?}");
    }
}

#[test]
fn refuses_a_schema_that_is_not_there() {
    let Ok(base) = std::env::var("CITYDB_TEST_URL") else {
        return;
    };
    let mut config = citygml_citydb::Config::new(format!("{base}/delft"));
    config.schema = "no_such_schema".into();
    let message = match citygml_citydb::CityDb::connect(&config) {
        Ok(_) => panic!("a missing schema must be refused"),
        Err(e) => e.to_string(),
    };
    assert!(message.contains("no_such_schema"), "{message}");
}

/// The CityJSON header names the database's reference system and steps by
/// its units: a millimetre for the 3DBAG's metres, a nanodegree for
/// PLATEAU's degrees.
#[cfg(feature = "cjseq")]
#[test]
fn the_header_steps_by_the_reference_system_s_units() {
    let Some(db) = common::database("delft") else {
        return;
    };
    let header = db.header().unwrap();
    assert_eq!(header.transform.scale, [0.001; 3]);
    let metadata = header
        .metadata
        .expect("a database with features has metadata");
    assert_eq!(
        metadata.reference_system.map(|rs| rs.to_url()).as_deref(),
        Some("https://www.opengis.net/def/crs/EPSG/0/7415")
    );
    assert_eq!(metadata.geographical_extent, db.extent().unwrap());
    let Some(db) = common::database("plateau_bldg") else {
        return;
    };
    assert_eq!(db.header().unwrap().transform.scale, [1e-9, 1e-9, 0.001]);
}

/// The translate is on the grid of the step: a database's extent is
/// computed (the railway scene's lower corner is no millimetre), and an
/// origin off the grid would move every stored coordinate by up to half a
/// step and merge vertices a step apart.
#[cfg(feature = "cjseq")]
#[test]
fn the_header_s_translate_is_a_whole_number_of_steps() {
    let Some(db) = common::database("railway_scene") else {
        return;
    };
    let header = db.header().unwrap();
    let extent = db.extent().unwrap().expect("an extent");
    let transform = &header.transform;
    for (axis, ((origin, step), lower)) in transform
        .translate
        .iter()
        .zip(transform.scale)
        .zip(extent)
        .enumerate()
    {
        let steps = origin / step;
        assert!(
            (steps - steps.round()).abs() < 1e-6,
            "axis {axis}: translate {origin} is {steps} steps of {step}"
        );
        assert!(*origin <= lower);
        assert!(lower - origin < step);
    }
    assert!(
        header.geometry_templates.is_some(),
        "the railway scene places templates"
    );
}
