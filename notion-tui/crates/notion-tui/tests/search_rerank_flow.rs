use std::sync::{Arc, Mutex};

use notion_store::{PageRec, Store};
use notion_tui::app::App;
use notion_tui::ui::search::SearchState;
use notion_tui::ui::textline::TextLine;

fn page(id: &str, title: &str) -> PageRec {
    PageRec {
        id: id.into(),
        parent_type: "workspace".into(),
        parent_id: None,
        title: title.into(),
        icon: None,
        archived: false,
        last_edited_time: "t".into(),
    }
}

#[test]
fn stronger_subsequence_match_outranks_earlier_fts_hit() {
    let s = Store::open_in_memory().unwrap();
    // FTS returns "The map" first for query "map", but "Mapping global
    // logistics" scores higher on the subsequence rank (80 vs 60), so
    // re-ranking must invert the FTS order.
    s.upsert_page(&page("p1", "The map")).unwrap();
    s.upsert_page(&page("p2", "Mapping global logistics")).unwrap();
    let store = Arc::new(Mutex::new(s));

    let mut app = App::new(store);
    app.search = Some(SearchState::new());
    app.search.as_mut().unwrap().input = TextLine::new("map");
    app.refresh_search();

    let titles: Vec<String> = app
        .search
        .as_ref()
        .unwrap()
        .results
        .iter()
        .map(|h| h.title.clone())
        .collect();
    assert_eq!(
        titles,
        vec!["Mapping global logistics", "The map"],
        "subsequence re-rank must invert FTS order without dropping hits"
    );
}
