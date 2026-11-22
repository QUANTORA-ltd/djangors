# Geospatial & PostGIS (`djangors-contrib-gis`)

`djangors-contrib-gis` is Djangors's GeoDjango equivalent. It provides pure-Rust geometry types,
WKT / EWKT parsing and serialization, great-circle (Haversine) distance calculation, and PostGIS
SQL predicate helpers.

## Installation

Enable the `gis` feature on the `djangors` facade crate (or depend on `djangors-contrib-gis` directly):

```toml
[dependencies]
djangors = { version = "1.0", features = ["gis"] }
```

## Geometry Types

| Type | Description | Example |
|---|---|---|
| `Point` | 2D `(x, y)` / `(lon, lat)` coordinate with SRID (default `4326` WGS84) | `Point::wgs84(-0.1276, 51.5072)` |
| `LineString` | Ordered sequence of `Point` vertices | `LineString::new(vec![a, b], 4326)` |
| `Polygon` | Exterior ring plus optional interior hole rings | `Polygon::new(exterior, vec![], 4326)` |
| `Geometry` | Enum wrapping `Point`, `LineString`, or `Polygon` | `Geometry::Point(p)` |

```rust,illustrative
use djangors_contrib_gis::{Geometry, LineString, Point, Polygon};

// WGS84 (SRID 4326): longitude first, latitude second (matches PostGIS & GeoJSON)
let london = Point::wgs84(-0.1276, 51.5072);
let paris = Point::wgs84(2.3522, 48.8566);

// Great-circle distance in meters (no database required)
let meters = london.haversine_distance_meters(&paris);
assert!(meters > 340_000.0 && meters < 346_000.0);

// Format as WKT or PostGIS EWKT
assert_eq!(london.to_wkt(), "POINT(-0.1276 51.5072)");
assert_eq!(london.to_ewkt(), "SRID=4326;POINT(-0.1276 51.5072)");

// Parse WKT or EWKT back into a Geometry
let parsed = Geometry::from_ewkt("SRID=4326;POINT(3.3792 6.5244)").unwrap();
assert_eq!(parsed.srid(), 4326);
```

## PostGIS SQL Helpers

`djangors_contrib_gis::postgis` generates parameterized PostGIS SQL expressions that pair with
`djangors-db` or `QuerySet::raw`:

```rust,illustrative
use djangors_contrib_gis::postgis;

// Column DDL helper
let col_ddl = postgis::column_type("Point", 4326);
// => "geometry(Point, 4326)"

// Spatial predicates using bind parameter placeholders ($1, $2, ...)
let within_5km = postgis::st_dwithin_meters("location", "$1", "$2");
let contains = postgis::st_contains("boundary", "$1");
let within = postgis::st_within("location", "$1");
let intersects = postgis::st_intersects("area", "$1");
let dist_expr = postgis::st_distance_meters("location", "$1");
```

Pass the target geometry as an EWKT string (`point.to_ewkt()`) for `$1` and the radius in meters
as a float for `$2`.
