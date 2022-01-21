use libs::divvy::ProgressTx;
use ogma::rt;
use std::path::Path;

fn paths() -> (&'static Path, &'static Path) {
    (Path::new("."), Path::new("../ogma"))
}

#[test]
fn batch_success_testing() {
    use rt::bat::*;
    use Outcome::*;

    let p = &ProgressTx::dummy();
    let (root, wd) = paths();

    let code = r#"def foo-bar () { \5 }

# Testing a comment
foo-bar | + 5

def-ty Foo { x:Num }"#;

    let batch = parse_str(code);
    let p = |b| {
        process(&b, root, wd, p, Default::default())
            .into_iter()
            .map(|x| x.0)
            .collect::<Vec<_>>()
    };
    assert_eq!(p(batch), vec![Success, Success, Success]);

    let batch = parse_str(code);
    assert_eq!(p(batch), vec![Success, Success, Success]);
}

#[test]
fn batch_fail_testing() {
    use rt::bat::*;
    use Outcome::*;

    let p = &ProgressTx::dummy();
    let (root, wd) = paths();

    let code = r#"def foo-bar () { \5 }

# This should fail
foo-bar | + 5 | - 'foo'

def-ty Foo { x:Num y: }"#;

    fn print<T>(o: (Outcome, T)) -> String {
        match o {
            (Failed(e), _) => e.to_string(),
            _ => panic!(),
        }
    }

    let batch = Batch {
        parallelise: false,
        fail_fast: true,
        ..parse_str(code)
    };
    let mut x = process(&batch, root, wd, p, Default::default()).into_iter();
    assert!(matches!(x.next(), Some((Success, _))));
    assert!(matches!(x.next(), Some((Outstanding, _))));
    assert_eq!(
        &x.next().map(print).unwrap(),
        "Parsing Error: could not parse input line
--> '' - line 6:21
 | def-ty Foo { x:Num y: }
 |                      ^ missing a valid type specifier: `field:Type`
"
    );

    let batch = parse_str(code);
    let mut x = process(&batch, root, wd, p, Default::default()).into_iter();
    assert!(matches!(x.next(), Some((Success, _))));
    assert_eq!(
        &x.next().map(print).unwrap(),
        r#"Semantics Error: expecting argument with type `Number`, found `String`
--> '' - line 4:19
 | foo-bar | + 5 | - 'foo'
 |                    ^^^ this argument returns type `String`
--> help: commands may require specific argument types, use `--help` to view requirements
"#
    );
    assert_eq!(
        &x.next().map(print).unwrap(),
        "Parsing Error: could not parse input line
--> '' - line 6:21
 | def-ty Foo { x:Num y: }
 |                      ^ missing a valid type specifier: `field:Type`
"
    );
}
