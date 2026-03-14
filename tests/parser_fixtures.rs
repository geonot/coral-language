use coralc::lexer::lex;
use coralc::parser::Parser;
use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn parser_valid_fixtures() {
    for fixture in collect_fixtures("tests/fixtures/parser/valid") {
        let source = fs::read_to_string(&fixture).expect("failed to read fixture");
        let tokens = lex(&source).expect("lexing failed");
        let parser = Parser::new(tokens, source.len());
        parser.parse().unwrap_or_else(|err| {
            panic!(
                "expected fixture {:?} to parse successfully, but got: {}",
                fixture, err.message
            );
        });
    }
}

#[test]
fn parser_invalid_fixtures() {
    for fixture in collect_fixtures("tests/fixtures/parser/invalid") {
        let expected = read_expectation(&fixture);
        let source = fs::read_to_string(&fixture).expect("failed to read fixture");
        let tokens = match lex(&source) {
            Ok(tokens) => tokens,
            Err(error) => {
                assert!(
                    error.message.contains(&expected)
                        || error
                            .help
                            .as_deref()
                            .map(|help| help.contains(&expected))
                            .unwrap_or(false),
                    "lex diagnostic `{}` did not contain expected substring `{}` for {:?}",
                    error.message,
                    expected,
                    fixture,
                );
                continue;
            }
        };
        let parser = Parser::new(tokens, source.len());
        let error = match parser.parse() {
            Ok(_) => panic!(
                "fixture {:?} should fail to parse, but it succeeded",
                fixture
            ),
            Err(err) => err,
        };
        assert!(
            error.message.contains(&expected)
                || error
                    .help
                    .as_deref()
                    .map(|help| help.contains(&expected))
                    .unwrap_or(false),
            "diagnostic `{}` did not contain expected substring `{}` for {:?}",
            error.message,
            expected,
            fixture,
        );
    }
}

fn collect_fixtures(dir: &str) -> Vec<PathBuf> {
    let mut entries = Vec::new();
    if let Ok(read_dir) = fs::read_dir(dir) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("coral") {
                entries.push(path);
            }
        }
    }
    entries.sort();
    entries
}

fn read_expectation(fixture: &Path) -> String {
    let mut expect_path = fixture.to_path_buf();
    expect_path.set_extension("expect");
    fs::read_to_string(&expect_path)
        .unwrap_or_else(|_| panic!("expected file {:?} to exist", expect_path))
        .trim()
        .to_string()
}
