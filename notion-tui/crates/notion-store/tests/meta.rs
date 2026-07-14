#[test]
fn meta_delete_removes_the_key() {
    let s = notion_store::Store::open_in_memory().unwrap();
    s.meta_set("k", "v").unwrap();
    assert_eq!(s.meta_get("k").unwrap().as_deref(), Some("v"));
    s.meta_delete("k").unwrap();
    assert_eq!(s.meta_get("k").unwrap(), None);
}
