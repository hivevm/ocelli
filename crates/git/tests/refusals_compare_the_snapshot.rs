//! Verifies: ADR-0031
//!
//! Every refusal the tests of the writes take goes through `refused` in `support/mod.rs`, which
//! compares the whole snapshot of the repository before and after, so that none compares less
//! (`docs/CONVENTIONS.md`, a refusal changes nothing).

/// The tests held to `refused`, by name and text.
const HELD: &[(&str, &str)] = &[
    ("write.rs", include_str!("write.rs")),
    ("history.rs", include_str!("history.rs")),
];

/// The lines of the Rust source `text` that take a refusal of a write, by number: an error taken
/// unconditionally, or one taken of a write on the line, the one before, or the one after.
fn refusals_outside_the_helper(text: &str) -> Vec<usize> {
    let lines: Vec<&str> = text.lines().collect();
    let writes = |l: &str| l.contains(".write(") || l.contains("write_reporting(");
    let near = |j: Option<usize>| j.and_then(|j| lines.get(j)).is_some_and(|l| writes(l));
    let mut found = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let takes_an_error = [
            concat!(".unwrap", "_err()"),
            concat!(".expect", "_err("),
            concat!(".is", "_err()"),
            concat!(".err", "()"),
            concat!("Err", "("),
        ]
        .iter()
        .any(|p| line.contains(p));
        let of_a_write = writes(line) || near(i.checked_sub(1)) || near(Some(i + 1));
        let unconditional = line.contains(concat!(".unwrap", "_err()"))
            || line.contains(concat!(".expect", "_err("));
        if takes_an_error && (unconditional || of_a_write) {
            found.push(i + 1);
        }
    }
    found
}

#[test]
fn every_refusal_test_of_the_writer_compares_the_snapshot() {
    let helper = include_str!("support/mod.rs");
    assert!(
        helper.contains("pub fn refused_reporting("),
        "the helper is where the tests take a refusal"
    );
    let mut found = Vec::new();
    for (name, text) in HELD {
        assert!(text.contains("mod support;"), "{name} reads the helper");
        found.extend(
            refusals_outside_the_helper(text)
                .into_iter()
                .map(|line| format!("crates/git/tests/{name}:{line}")),
        );
    }
    assert!(
        found.is_empty(),
        "a refusal taken outside refused() on {found:?}"
    );
}

#[test]
fn the_check_names_each_line_that_takes_a_refusal_outside_the_helper() {
    let unwrap = concat!(".unwrap", "_err()");
    let expect = concat!(".expect", "_err(\"refused\")");
    let text = format!(
        "fn a() {{\n    let e = GitWriter.write(d, &w){unwrap};\n}}\n\
         let f = w(){expect};\n"
    );
    assert_eq!(refusals_outside_the_helper(&text), vec![2, 4]);
    assert_eq!(
        refusals_outside_the_helper("fn a() {\n    refused(d, &w, \"why\");\n}\n"),
        Vec::<usize>::new()
    );
    let is_err = concat!(".is", "_err()");
    let let_err = concat!("let ", "Err(e)");
    let err_arm = concat!("Err", "(e) => {}");
    let text = format!(
        "assert!(GitWriter.write(d, &w){is_err});\n\
         {let_err} = GitWriter\n    .write(d, &w) else {{ return }};\n\
         let r = GitWriter.write_reporting(d, &w, &mut go);\nif {let_err} = r {{}}\n\
         assert!(std::fs::read(p){is_err});\n\
         fn b() {{}}\nmatch GitWriter.write(d, &w) {{\n    {err_arm}\n}}\n\
         match std::fs::read(p) {{\n    Ok(_) => {{}}\n    {err_arm}\n}}\n"
    );
    assert_eq!(refusals_outside_the_helper(&text), vec![1, 2, 5, 9]);
}
