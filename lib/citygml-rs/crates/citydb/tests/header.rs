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
