use super::*;

/// Grid and edge inputs: halves, near-integers, subnormals, saturating magnitudes, NaN and infinities.
fn samples() -> Vec<f64> {
    let mut values = vec![
        0.0,
        -0.0,
        0.5,
        -0.5,
        1.0,
        -1.0,
        f64::MIN_POSITIVE,
        -f64::MIN_POSITIVE,
        2_147_483_647.0,
        2_147_483_647.5,
        2_147_483_648.0,
        -2_147_483_648.0,
        -2_147_483_648.5,
        -2_147_483_649.0,
        3.0e9,
        -3.0e9,
        1.0e300,
        -1.0e300,
        f64::NAN,
        f64::from(f32::from_bits(1)),
        f64::from(f32::from_bits(1 << 23)),
        16_777_216.0,
        16_777_217.0,
        2_147_483_520.0,
        f64::from(1.0e37_f32),
        -f64::from(1.0e37_f32),
        f64::INFINITY,
        f64::NEG_INFINITY,
    ];
    for step in -70_000..70_000 {
        let base = f64::from(step) / 8.0;
        values.extend([base, base + 1e-12, base - 1e-12, base * 4096.0 / 3.0]);
    }
    values
}

#[test]
fn floor_matches_the_libm_form() {
    for value in samples() {
        assert_eq!(
            floor_i32(value),
            value.floor() as i32,
            "f64 {value:e} floors differently"
        );
        let narrow = value as f32;
        assert_eq!(
            floor_f32_i32(narrow),
            narrow.floor() as i32,
            "f32 {narrow:e} floors differently"
        );
        assert_eq!(
            pack_degrees(narrow),
            (narrow.mul_add(256.0, 0.0) / 360.0).floor() as i32 as u8,
            "angle {narrow:e} packs differently"
        );
    }
}
