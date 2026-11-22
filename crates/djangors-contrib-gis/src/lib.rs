#![deny(missing_docs)]
//! GeoDjango-equivalent geospatial types and PostGIS SQL helpers for Djangors.
//!
//! Provides WGS-84 (SRID 4326) and arbitrary-SRID geometry types (`Point`, `LineString`,
//! `Polygon`, `Geometry`), WKT/EWKT serialization and parsing, Haversine great-circle
//! distance computation, and PostGIS spatial SQL predicate builders.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Default WGS-84 Spatial Reference System Identifier (latitude/longitude).
pub const SRID_WGS84: u32 = 4326;

/// Errors returned when parsing or validating geometries.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum GisError {
    /// WKT/EWKT text could not be parsed.
    #[error("invalid WKT geometry: {0}")]
    InvalidWkt(String),
    /// A polygon ring has fewer than 4 points or is not closed.
    #[error("invalid polygon ring: {0}")]
    InvalidPolygon(String),
}

/// A 2D geographic/geometric point (`(x, y)` = `(longitude, latitude)` in SRID 4326).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    /// X coordinate (longitude in WGS-84).
    pub x: f64,
    /// Y coordinate (latitude in WGS-84).
    pub y: f64,
    /// Spatial Reference System Identifier (defaults to 4326).
    pub srid: u32,
}

impl Point {
    /// Creates a WGS-84 (SRID 4326) point from `(longitude, latitude)`.
    pub fn new(x: f64, y: f64) -> Self {
        Self {
            x,
            y,
            srid: SRID_WGS84,
        }
    }

    /// Creates a point with an explicit SRID.
    pub fn with_srid(x: f64, y: f64, srid: u32) -> Self {
        Self { x, y, srid }
    }

    /// Formats as standard WKT: `POINT(x y)`.
    pub fn to_wkt(&self) -> String {
        format!("POINT({} {})", self.x, self.y)
    }

    /// Formats as PostGIS EWKT: `SRID=4326;POINT(x y)`.
    pub fn to_ewkt(&self) -> String {
        format!("SRID={};{}", self.srid, self.to_wkt())
    }

    /// Parses `POINT(x y)` or `SRID=4326;POINT(x y)`.
    pub fn from_wkt(input: &str) -> Result<Self, GisError> {
        let (srid, body) = split_ewkt(input)?;
        let inner = strip_type_parens(body, "POINT")?;
        let (x, y) = parse_coord_pair(inner)?;
        Ok(Self { x, y, srid })
    }

    /// Computes great-circle Haversine distance in meters between two WGS-84 points.
    pub fn haversine_distance_meters(&self, other: &Point) -> f64 {
        const EARTH_RADIUS_M: f64 = 6_371_008.8;
        let lat1 = self.y.to_radians();
        let lat2 = other.y.to_radians();
        let dlat = (other.y - self.y).to_radians();
        let dlon = (other.x - self.x).to_radians();

        let a = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
        let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
        EARTH_RADIUS_M * c
    }
}

/// A 2D polyline composed of ordered `(x, y)` vertices.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LineString {
    /// Ordered vertices.
    pub points: Vec<(f64, f64)>,
    /// Spatial Reference System Identifier.
    pub srid: u32,
}

impl LineString {
    /// Creates a WGS-84 `LineString`.
    pub fn new(points: Vec<(f64, f64)>) -> Self {
        Self {
            points,
            srid: SRID_WGS84,
        }
    }

    /// Formats as WKT: `LINESTRING(x1 y1, x2 y2, ...)`.
    pub fn to_wkt(&self) -> String {
        let coords: Vec<String> = self
            .points
            .iter()
            .map(|(x, y)| format!("{x} {y}"))
            .collect();
        format!("LINESTRING({})", coords.join(", "))
    }

    /// Formats as PostGIS EWKT.
    pub fn to_ewkt(&self) -> String {
        format!("SRID={};{}", self.srid, self.to_wkt())
    }

    /// Parses `LINESTRING(...)` or `SRID=...;LINESTRING(...)`.
    pub fn from_wkt(input: &str) -> Result<Self, GisError> {
        let (srid, body) = split_ewkt(input)?;
        let inner = strip_type_parens(body, "LINESTRING")?;
        let points = parse_coord_list(inner)?;
        Ok(Self { points, srid })
    }
}

/// A 2D polygon defined by an exterior ring and optional interior hole rings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Polygon {
    /// Rings of `(x, y)` coordinates; the first ring is the exterior boundary.
    pub rings: Vec<Vec<(f64, f64)>>,
    /// Spatial Reference System Identifier.
    pub srid: u32,
}

impl Polygon {
    /// Creates a WGS-84 polygon from an exterior ring (validating closure and length >= 4).
    pub fn new(exterior: Vec<(f64, f64)>) -> Result<Self, GisError> {
        validate_ring(&exterior)?;
        Ok(Self {
            rings: vec![exterior],
            srid: SRID_WGS84,
        })
    }

    /// Formats as WKT: `POLYGON((x1 y1, ...))`.
    pub fn to_wkt(&self) -> String {
        let rings: Vec<String> = self
            .rings
            .iter()
            .map(|ring| {
                let pts: Vec<String> = ring.iter().map(|(x, y)| format!("{x} {y}")).collect();
                format!("({})", pts.join(", "))
            })
            .collect();
        format!("POLYGON({})", rings.join(", "))
    }

    /// Formats as PostGIS EWKT.
    pub fn to_ewkt(&self) -> String {
        format!("SRID={};{}", self.srid, self.to_wkt())
    }

    /// Point-in-polygon ray-casting test against the exterior ring (ignoring holes).
    pub fn contains_point(&self, pt: Point) -> bool {
        let Some(ring) = self.rings.first() else {
            return false;
        };
        let mut inside = false;
        let mut j = ring.len().saturating_sub(1);
        for i in 0..ring.len() {
            let (xi, yi) = ring[i];
            let (xj, yj) = ring[j];
            let intersects =
                ((yi > pt.y) != (yj > pt.y)) && (pt.x < (xj - xi) * (pt.y - yi) / (yj - yi) + xi);
            if intersects {
                inside = !inside;
            }
            j = i;
        }
        inside
    }
}

/// Unified geometry enum.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Geometry {
    /// A 2D point.
    Point(Point),
    /// A 2D linestring.
    LineString(LineString),
    /// A 2D polygon.
    Polygon(Polygon),
}

impl Geometry {
    /// Formats this geometry as PostGIS EWKT.
    pub fn to_ewkt(&self) -> String {
        match self {
            Geometry::Point(p) => p.to_ewkt(),
            Geometry::LineString(l) => l.to_ewkt(),
            Geometry::Polygon(p) => p.to_ewkt(),
        }
    }
}

/// PostGIS spatial predicate helpers generating SQL fragments for PostgreSQL + PostGIS.
pub mod postgis {
    use super::Geometry;

    /// Generates `ST_DWithin("col"::geography, ST_GeogFromText('SRID=...'), meters)`.
    pub fn st_dwithin_meters(column: &str, target: &Geometry, meters: f64) -> String {
        let ewkt = target.to_ewkt().replace('\'', "''");
        format!("ST_DWithin(\"{column}\"::geography, ST_GeogFromText('{ewkt}'), {meters})")
    }

    /// Generates `ST_Contains("col", ST_GeomFromEWKT('...'))`.
    pub fn st_contains(column: &str, target: &Geometry) -> String {
        let ewkt = target.to_ewkt().replace('\'', "''");
        format!("ST_Contains(\"{column}\", ST_GeomFromEWKT('{ewkt}'))")
    }

    /// Generates `ST_Within("col", ST_GeomFromEWKT('...'))`.
    pub fn st_within(column: &str, target: &Geometry) -> String {
        let ewkt = target.to_ewkt().replace('\'', "''");
        format!("ST_Within(\"{column}\", ST_GeomFromEWKT('{ewkt}'))")
    }

    /// Generates `ST_Intersects("col", ST_GeomFromEWKT('...'))`.
    pub fn st_intersects(column: &str, target: &Geometry) -> String {
        let ewkt = target.to_ewkt().replace('\'', "''");
        format!("ST_Intersects(\"{column}\", ST_GeomFromEWKT('{ewkt}'))")
    }

    /// Generates `ST_Distance("col"::geography, ST_GeogFromText('...'))` (meters).
    pub fn st_distance_meters(column: &str, target: &Geometry) -> String {
        let ewkt = target.to_ewkt().replace('\'', "''");
        format!("ST_Distance(\"{column}\"::geography, ST_GeogFromText('{ewkt}'))")
    }

    /// Generates PostGIS column DDL (e.g. `geometry(Point, 4326)`).
    pub fn column_type(geom_type: &str, srid: u32) -> String {
        format!("geometry({geom_type}, {srid})")
    }
}

fn split_ewkt(input: &str) -> Result<(u32, &str), GisError> {
    let trimmed = input.trim();
    if let Some(rest) = trimmed.strip_prefix("SRID=") {
        let (srid_str, geom) = rest
            .split_once(';')
            .ok_or_else(|| GisError::InvalidWkt("missing ';' after SRID".into()))?;
        let srid: u32 = srid_str
            .trim()
            .parse()
            .map_err(|_| GisError::InvalidWkt(format!("invalid SRID `{srid_str}`")))?;
        Ok((srid, geom.trim()))
    } else {
        Ok((SRID_WGS84, trimmed))
    }
}

fn strip_type_parens<'a>(body: &'a str, expected: &str) -> Result<&'a str, GisError> {
    let upper = body.to_ascii_uppercase();
    if !upper.starts_with(expected) {
        return Err(GisError::InvalidWkt(format!("expected {expected}")));
    }
    let rest = body[expected.len()..].trim();
    rest.strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .map(str::trim)
        .ok_or_else(|| GisError::InvalidWkt("missing parentheses".into()))
}

fn parse_coord_pair(s: &str) -> Result<(f64, f64), GisError> {
    let mut parts = s.split_whitespace();
    let x = parts
        .next()
        .ok_or_else(|| GisError::InvalidWkt("missing x coordinate".into()))?
        .parse::<f64>()
        .map_err(|_| GisError::InvalidWkt("invalid x coordinate".into()))?;
    let y = parts
        .next()
        .ok_or_else(|| GisError::InvalidWkt("missing y coordinate".into()))?
        .parse::<f64>()
        .map_err(|_| GisError::InvalidWkt("invalid y coordinate".into()))?;
    if parts.next().is_some() {
        return Err(GisError::InvalidWkt("unexpected trailing tokens".into()));
    }
    Ok((x, y))
}

fn parse_coord_list(s: &str) -> Result<Vec<(f64, f64)>, GisError> {
    s.split(',').map(|p| parse_coord_pair(p.trim())).collect()
}

fn validate_ring(ring: &[(f64, f64)]) -> Result<(), GisError> {
    if ring.len() < 4 {
        return Err(GisError::InvalidPolygon(
            "ring must contain at least 4 coordinates".into(),
        ));
    }
    if ring.first() != ring.last() {
        return Err(GisError::InvalidPolygon(
            "ring must be closed (first coordinate == last coordinate)".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_wkt_roundtrip_and_haversine() {
        let lagos = Point::new(3.3792, 6.5244);
        assert_eq!(lagos.to_wkt(), "POINT(3.3792 6.5244)");
        assert_eq!(lagos.to_ewkt(), "SRID=4326;POINT(3.3792 6.5244)");
        assert_eq!(Point::from_wkt(&lagos.to_ewkt()).unwrap(), lagos);

        let abuja = Point::new(7.4951, 9.0579);
        let dist_km = lagos.haversine_distance_meters(&abuja) / 1000.0;
        assert!(dist_km > 520.0 && dist_km < 550.0, "dist_km={dist_km}");
    }

    #[test]
    fn linestring_and_polygon_and_postgis_sql() {
        let line = LineString::new(vec![(0.0, 0.0), (1.0, 1.0)]);
        assert_eq!(LineString::from_wkt(&line.to_ewkt()).unwrap(), line);

        let square = Polygon::new(vec![
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
            (0.0, 0.0),
        ])
        .unwrap();
        assert!(square.contains_point(Point::new(5.0, 5.0)));
        assert!(!square.contains_point(Point::new(15.0, 5.0)));

        let sql =
            postgis::st_dwithin_meters("location", &Geometry::Point(Point::new(3.0, 6.0)), 500.0);
        assert!(sql.contains("ST_DWithin(\"location\"::geography"));
        assert!(sql.contains("SRID=4326;POINT(3 6)"));
    }
}
