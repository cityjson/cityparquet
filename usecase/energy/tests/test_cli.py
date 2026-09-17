from energy.cli import build_parser


def test_features_flags_parse():
    args = build_parser().parse_args(
        ["features", "--input", "in/*.parquet", "--lod", "2.2",
         "--output", "f.parquet", "--faces", "faces.parquet",
         "--validate", "report.json", "--flat-tilt-deg", "7.5"]
    )
    assert args.command == "features"
    assert args.input == "in/*.parquet"
    assert args.lod == "2.2"
    assert args.output == "f.parquet"
    assert args.faces == "faces.parquet"
    assert args.validate == "report.json"
    assert args.flat_tilt_deg == 7.5


def test_features_defaults():
    args = build_parser().parse_args(["features", "--input", "x.parquet"])
    assert args.lod == "2.2"
    assert args.output == "features.parquet"
    assert args.faces is None
    assert args.validate is None
    assert args.flat_tilt_deg == 5.0
    assert args.ext_dir is None


def test_screen_flags_parse():
    args = build_parser().parse_args(
        ["screen", "--features", "f.parquet", "--hdd", "3000",
         "--params", "u.toml", "--year-before", "1975",
         "--sv-above", "0.8", "--top", "100", "--output", "s.parquet"]
    )
    assert args.command == "screen"
    assert args.hdd == 3000.0
    assert args.year_before == 1975
    assert args.sv_above == 0.8
    assert args.top == 100


def test_screen_defaults():
    args = build_parser().parse_args(["screen", "--features", "f.parquet"])
    assert args.hdd == 2900.0
    assert args.params is None
    assert args.year_before is None
    assert args.sv_above is None
    assert args.top is None
    assert args.output == "screen.parquet"
