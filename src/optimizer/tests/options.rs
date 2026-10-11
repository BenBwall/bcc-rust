//! Pass-list parsing and the report's transformation counter.

use super::*;

#[test]
fn pass_lists_parse() {
    let list = parse_pass_list("fold,dce, simplify-cfg").unwrap();
    assert_eq!(list.as_slice(), [Pass::Fold, Pass::Dce, Pass::SimplifyCfg]);
    let repeated = parse_pass_list("dce,dce,fold").unwrap();
    assert_eq!(repeated.as_slice(), [Pass::Dce, Pass::Dce, Pass::Fold]);
    assert_eq!(parse_pass_list("").unwrap().as_slice(), []);
}

#[test]
fn bad_pass_lists_are_rejected() {
    assert_eq!(
        parse_pass_list("fold,gvn"),
        Err(PassListError::Unknown("gvn"))
    );
    assert_eq!(
        parse_pass_list("fold,,dce"),
        Err(PassListError::Unknown(""))
    );
    assert_eq!(parse_pass_list("fold,"), Err(PassListError::Unknown("")));
    let long = ["dce"; PassList::CAPACITY + 1].join(",");
    assert_eq!(parse_pass_list(&long), Err(PassListError::TooMany));
    let exact = ["dce"; PassList::CAPACITY].join(",");
    assert_eq!(
        parse_pass_list(&exact).unwrap().as_slice().len(),
        PassList::CAPACITY
    );
}

#[test]
fn the_default_pipeline_lists_every_pass_once() {
    let mut seen = [false; Pass::COUNT];
    for pass in Pass::ALL {
        assert!(!seen[pass.index()], "{pass} is listed twice");
        seen[pass.index()] = true;
        assert_eq!(Pass::from_name(pass.name()), Some(pass));
    }
    assert!(seen.iter().all(|&listed| listed));
}

#[test]
fn the_report_grants_exactly_the_limit() {
    let mut report = OptimizationReport::new(Some(2));
    report.begin_pass(Pass::Fold);
    assert!(report.allow());
    assert!(report.allow());
    assert!(!report.limit_reached());
    assert!(!report.allow());
    assert!(!report.allow());
    assert!(report.limit_reached());
    assert_eq!(report.transformations(), 2);
    assert_eq!(report.stats(Pass::Fold).changes, 2);
    let mut unlimited = OptimizationReport::new(None);
    assert!((0..1000).all(|_| unlimited.allow()));
    assert_eq!(unlimited.transformations(), 1000);
}
