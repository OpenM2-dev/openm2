//! 4×4 matrices in f64, column-major, for nodes' rest transforms: what
//! the reader needs to place meshes and to invert bones' world matrices.

/// A matrix, column-major (translation in 12 to 14).
pub type M4 = [f64; 16];

/// The identity.
pub const IDENTITY: M4 = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

fn at(m: &M4, row: usize, col: usize) -> f64 {
    m.get(col * 4 + row).copied().unwrap_or(0.0)
}

/// `a × b`.
pub fn mul(a: &M4, b: &M4) -> M4 {
    let mut out = [0.0; 16];
    for col in 0..4 {
        for row in 0..4 {
            let value = (0..4).map(|k| at(a, row, k) * at(b, k, col)).sum();
            if let Some(slot) = out.get_mut(col * 4 + row) {
                *slot = value;
            }
        }
    }
    out
}

/// From f32s.
pub fn from_f32(m: &[f32; 16]) -> M4 {
    m.map(f64::from)
}

/// To f32s.
pub fn to_f32(m: &M4) -> [f32; 16] {
    m.map(|v| v as f32)
}

/// Translation × rotation (a quaternion, normalised) × a 3×3 matrix given
/// by rows.
pub fn from_trs(t: [f32; 3], q: [f32; 4], m: [[f32; 3]; 3]) -> M4 {
    let [x, y, z, w] = q.map(f64::from);
    let length = (x * x + y * y + z * z + w * w).sqrt();
    let (x, y, z, w) = if length > 1e-12 && length.is_finite() {
        (x / length, y / length, z / length, w / length)
    } else {
        (0.0, 0.0, 0.0, 1.0)
    };
    let r = [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - z * w),
            2.0 * (x * z + y * w),
        ],
        [
            2.0 * (x * y + z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - x * w),
        ],
        [
            2.0 * (x * z - y * w),
            2.0 * (y * z + x * w),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ];
    let mut out = IDENTITY;
    for row in 0..3 {
        for col in 0..3 {
            let value = (0..3)
                .map(|k| {
                    let rk = r.get(row).and_then(|r| r.get(k)).copied().unwrap_or(0.0);
                    let mk = m.get(k).and_then(|m| m.get(col)).copied().unwrap_or(0.0);
                    rk * f64::from(mk)
                })
                .sum();
            if let Some(slot) = out.get_mut(col * 4 + row) {
                *slot = value;
            }
        }
    }
    for (row, value) in t.iter().enumerate() {
        if let Some(slot) = out.get_mut(12 + row) {
            *slot = f64::from(*value);
        }
    }
    out
}

/// The inverse, if there is one.
pub fn inverse(m: &M4) -> Option<M4> {
    // Gauss-Jordan elimination with partial pivoting on [m | I].
    let mut a = [[0.0f64; 8]; 4];
    for (row, line) in a.iter_mut().enumerate() {
        for col in 0..4 {
            if let Some(slot) = line.get_mut(col) {
                *slot = at(m, row, col);
            }
            if let Some(slot) = line.get_mut(4 + col) {
                *slot = if row == col { 1.0 } else { 0.0 };
            }
        }
    }
    for col in 0..4 {
        let pivot = (col..4).max_by(|&i, &j| {
            let vi = a
                .get(i)
                .and_then(|r| r.get(col))
                .copied()
                .unwrap_or(0.0)
                .abs();
            let vj = a
                .get(j)
                .and_then(|r| r.get(col))
                .copied()
                .unwrap_or(0.0)
                .abs();
            vi.total_cmp(&vj)
        })?;
        a.swap(col, pivot);
        let line = *a.get(col)?;
        let p = *line.get(col)?;
        if p.abs() < 1e-300 || !p.is_finite() {
            return None;
        }
        let line = line.map(|v| v / p);
        if let Some(slot) = a.get_mut(col) {
            *slot = line;
        }
        for row in 0..4 {
            if row == col {
                continue;
            }
            let Some(other) = a.get_mut(row) else {
                continue;
            };
            let factor = other.get(col).copied().unwrap_or(0.0);
            for (o, l) in other.iter_mut().zip(line) {
                *o -= factor * l;
            }
        }
    }
    let mut out = [0.0; 16];
    for (row, line) in a.iter().enumerate() {
        for col in 0..4 {
            if let Some(slot) = out.get_mut(col * 4 + row) {
                *slot = line.get(4 + col).copied().unwrap_or(0.0);
            }
        }
    }
    out.iter().all(|v| v.is_finite()).then_some(out)
}

/// A point moved by a matrix.
pub fn transform_point(m: &M4, p: [f32; 3]) -> [f32; 3] {
    let p = p.map(f64::from);
    std::array::from_fn(|row| {
        let moved: f64 = (0..3)
            .map(|k| at(m, row, k) * p.get(k).copied().unwrap_or(0.0))
            .sum();
        (moved + at(m, row, 3)) as f32
    })
}

/// The matrix normals move by: the inverse transpose of the 3×3 part (rows).
pub fn normal_matrix(m: &M4) -> [[f64; 3]; 3] {
    let inverse = inverse(m).unwrap_or(IDENTITY);
    std::array::from_fn(|row| std::array::from_fn(|col| at(&inverse, col, row)))
}

/// A normal moved by a normal matrix, then made unit length again.
pub fn transform_normal(m: &[[f64; 3]; 3], n: [f32; 3]) -> [f32; 3] {
    let n = n.map(f64::from);
    let moved: [f64; 3] = std::array::from_fn(|row| {
        (0..3)
            .map(|k| {
                m.get(row).and_then(|r| r.get(k)).copied().unwrap_or(0.0)
                    * n.get(k).copied().unwrap_or(0.0)
            })
            .sum()
    });
    let length = moved.iter().map(|v| v * v).sum::<f64>().sqrt();
    if length > 1e-12 {
        moved.map(|v| (v / length) as f32)
    } else {
        moved.map(|v| v as f32)
    }
}
