#[cfg(not(feature = "no-entrypoint"))]
#[test]
fn exported_security_metadata_has_live_policy_and_private_reporting_links() {
    let value = holdnow::SECURITY_TXT;
    let begin = "=======BEGIN SECURITY.TXT V1=======\0";
    let end = "=======END SECURITY.TXT V1=======\0";
    let body = value
        .strip_prefix(begin)
        .unwrap()
        .strip_suffix(end)
        .unwrap();
    let fields: Vec<_> = body.split_terminator('\0').collect();
    assert_eq!(fields.len() % 2, 0);
    let pairs: std::collections::BTreeMap<_, _> =
        fields.chunks_exact(2).map(|p| (p[0], p[1])).collect();
    assert_eq!(pairs.len() * 2, fields.len(), "Duplicate metadata field");
    assert_eq!(pairs["name"], "Hold.now");
    assert_eq!(pairs["project_url"], "https://hold.now");
    assert_eq!(
        pairs["contacts"],
        "link:https://github.com/holdnow/contracts-release/security/advisories/new"
    );
    assert_eq!(
        pairs["policy"],
        "https://github.com/holdnow/contracts-release/blob/main/SECURITY.md"
    );
    assert_eq!(
        pairs["source_code"],
        "https://github.com/holdnow/contracts-release"
    );
    assert_eq!(pairs["preferred_languages"], "en");
}
