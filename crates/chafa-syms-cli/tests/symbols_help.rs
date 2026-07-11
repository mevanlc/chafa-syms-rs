use std::process::Command;

#[test]
fn symbols_help_succeeds_without_an_image() {
    let output = Command::new(env!("CARGO_BIN_EXE_chafa-syms"))
        .args(["--symbols", "help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(output.stderr.is_empty());

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("NAME\n"));
    assert!(stdout.contains("GRAMMAR\n"));
    assert!(stdout.contains("NAMED SYMBOL SETS\n"));
    assert!(stdout.contains("EXAMPLES\n"));

    let named_sets = [
        "all",
        "none",
        "space",
        "solid",
        "stipple",
        "block",
        "border",
        "diagonal",
        "dot",
        "quad",
        "half",
        "hhalf",
        "vhalf",
        "inverted",
        "braille",
        "sextant",
        "octant",
        "wedge",
        "technical",
        "geometric",
        "ascii",
        "alpha",
        "digit",
        "alnum",
        "narrow",
        "wide",
        "ambiguous",
        "ugly",
        "bad",
        "legacy",
        "latin",
        "import",
        "imported",
        "extra",
    ];
    for name in named_sets {
        let prefix = format!("    {name:<12}");
        assert!(
            stdout.lines().any(|line| line.starts_with(&prefix)),
            "missing description for named set '{name}'"
        );
    }
}

#[test]
fn non_help_symbol_spec_still_requires_an_image() {
    let output = Command::new(env!("CARGO_BIN_EXE_chafa-syms"))
        .args(["--symbols", "ascii"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("required arguments were not provided"));
}

#[test]
fn symbols_help_accepts_equals_syntax() {
    let output = Command::new(env!("CARGO_BIN_EXE_chafa-syms"))
        .arg("--symbols=help")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(output.stdout.starts_with(b"NAME\n"));
}
