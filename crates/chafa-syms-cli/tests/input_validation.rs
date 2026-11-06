use std::process::Command;

#[test]
fn invalid_geometry_reports_an_error_without_rendering() {
    let path =
        std::env::temp_dir().join(format!("chafa-syms-validation-{}.png", std::process::id()));
    image::save_buffer(&path, &[255; 4], 1, 1, image::ColorType::Rgba8).unwrap();
    for (flag, value) in [
        ("--size", "80xjunk"),
        ("--size", "99999999999999999999999x1"),
        ("--size", "8192x1"),
        ("--font-ratio", "NaN"),
        ("--font-ratio", "inf"),
        ("--scale", "NaN"),
        ("--scale", "inf"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_chafa-syms"))
            .args([flag, value])
            .arg(&path)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{flag} {value}");
        assert!(output.stdout.is_empty(), "{flag} {value}");
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains(flag), "{flag} {value}: {error}");
    }
    std::fs::remove_file(path).unwrap();
}
