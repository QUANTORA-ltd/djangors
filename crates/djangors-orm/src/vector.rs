//! Vector embedding type and distance metrics for `pgvector` and portable SQLite similarity search.

use std::fmt;
use std::str::FromStr;

/// Distance metric used for vector similarity ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum VectorDistanceMetric {
    /// Cosine distance (`<=>` in pgvector, `1 - cosine_similarity`).
    Cosine,
    /// Euclidean / L2 distance (`<->` in pgvector).
    L2,
    /// Negative inner product (`<#>` in pgvector).
    InnerProduct,
}

impl VectorDistanceMetric {
    /// Returns the pgvector SQL operator for this metric.
    pub fn pgvector_operator(self) -> &'static str {
        match self {
            Self::Cosine => "<=>",
            Self::L2 => "<->",
            Self::InnerProduct => "<#>",
        }
    }
}

/// Dense `f32` embedding vector stored as `VECTOR(N)` on PostgreSQL (`pgvector`)
/// and `[x1,x2,...]` `TEXT` on SQLite.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Vector(pub Vec<f32>);

impl Vector {
    /// Create a [`Vector`] from a vector of `f32` components.
    pub fn new(values: Vec<f32>) -> Self {
        Self(values)
    }

    /// Borrow the underlying `f32` slice.
    pub fn as_slice(&self) -> &[f32] {
        &self.0
    }

    /// Return the number of dimensions in this vector.
    pub fn dimensions(&self) -> usize {
        self.0.len()
    }

    /// Format this vector as a pgvector literal string `"[0.1,0.2,...]"`.
    pub fn to_pgvector_literal(&self) -> String {
        let parts: Vec<String> = self
            .0
            .iter()
            .map(|v| {
                if v.is_finite() {
                    v.to_string()
                } else {
                    "0".to_string()
                }
            })
            .collect();
        format!("[{}]", parts.join(","))
    }

    /// Parse a pgvector literal string `"[0.1,0.2,...]"` into a [`Vector`].
    pub fn from_pgvector_literal(raw: &str) -> Result<Self, String> {
        let trimmed = raw.trim();
        let inner = trimmed
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .ok_or_else(|| format!("invalid vector literal '{trimmed}': expected [...]"))?;
        if inner.trim().is_empty() {
            return Ok(Self(Vec::new()));
        }
        let mut values = Vec::new();
        for part in inner.split(',') {
            let v = part
                .trim()
                .parse::<f32>()
                .map_err(|e| format!("invalid vector element '{part}': {e}"))?;
            if !v.is_finite() {
                return Err(format!("non-finite vector element '{part}'"));
            }
            values.push(v);
        }
        Ok(Self(values))
    }

    /// Compute cosine distance (`1.0 - cosine_similarity`) between `self` and `other`.
    pub fn cosine_distance(&self, other: &Self) -> f32 {
        let mut dot = 0.0_f32;
        let mut norm_a = 0.0_f32;
        let mut norm_b = 0.0_f32;
        for (a, b) in self.0.iter().zip(other.0.iter()) {
            dot += a * b;
            norm_a += a * a;
            norm_b += b * b;
        }
        let denom = norm_a.sqrt() * norm_b.sqrt();
        if denom == 0.0 {
            1.0
        } else {
            1.0 - (dot / denom)
        }
    }

    /// Compute Euclidean (L2) distance between `self` and `other`.
    pub fn l2_distance(&self, other: &Self) -> f32 {
        self.0
            .iter()
            .zip(other.0.iter())
            .map(|(a, b)| {
                let d = a - b;
                d * d
            })
            .sum::<f32>()
            .sqrt()
    }

    /// Compute negative inner product (`-sum(a_i * b_i)`) matching pgvector `<#>`.
    pub fn inner_product(&self, other: &Self) -> f32 {
        -self
            .0
            .iter()
            .zip(other.0.iter())
            .map(|(a, b)| a * b)
            .sum::<f32>()
    }

    /// Compute distance under the requested [`VectorDistanceMetric`].
    pub fn distance(&self, other: &Self, metric: VectorDistanceMetric) -> f32 {
        match metric {
            VectorDistanceMetric::Cosine => self.cosine_distance(other),
            VectorDistanceMetric::L2 => self.l2_distance(other),
            VectorDistanceMetric::InnerProduct => self.inner_product(other),
        }
    }
}

impl From<Vec<f32>> for Vector {
    fn from(values: Vec<f32>) -> Self {
        Self(values)
    }
}

impl FromStr for Vector {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_pgvector_literal(s)
    }
}

impl TryFrom<&str> for Vector {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::from_pgvector_literal(value)
    }
}

impl fmt::Display for Vector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_pgvector_literal())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_roundtrip_and_metrics() {
        let a = Vector::new(vec![1.0, 0.0, 0.0]);
        let b = Vector::new(vec![0.0, 1.0, 0.0]);
        let parsed: Vector = a.to_pgvector_literal().parse().unwrap();
        assert_eq!(a, parsed);
        assert!((a.cosine_distance(&a)).abs() < 1e-6);
        assert!((a.cosine_distance(&b) - 1.0).abs() < 1e-6);
        assert!((a.l2_distance(&b) - std::f32::consts::SQRT_2).abs() < 1e-5);
        assert!((a.inner_product(&a) + 1.0).abs() < 1e-6);
    }
}
