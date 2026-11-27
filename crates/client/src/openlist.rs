//! The open Track list (D81): the Track lists received so far and each one's view with its sort, filter, selection
//! and scroll, kept so going back finds a list as you left it. Which list shows is the App's page (D88).
//!
//! Rows are known by identity ([`RowId`]: the playlist row's uid, else its track URI), never by where they show: a
//! list changes only through [`OpenList::edit`], and when the edit is done the selection is back on the row it was
//! on, on the same line of the screen (a page arriving into a sorted list used to move it to another song). If that
//! row is gone, the row that took its place is selected.

use crate::tracks::{TrackList, TrackView};
use singularity_proto::{ListHeader, TrackPage, TrackRow};
use std::collections::HashMap;
use std::rc::Rc;

/// A row of a Track list by identity: its playlist uid, else its track URI (albums and Liked Songs have no uids, and
/// don't hold a track twice).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RowId(String);

impl RowId {
    pub fn of(row: &TrackRow) -> RowId {
        RowId(row.uid.clone().unwrap_or_else(|| row.uri.clone()))
    }
}

#[derive(Default)]
pub struct OpenList {
    lists: HashMap<String, TrackList>,
    /// each list's view, the one shown and the ones left (which one shows is the App's page, D88)
    views: HashMap<String, TrackView>,
}

impl OpenList {
    // ── the lists ──

    pub fn list(&self, uri: &str) -> Option<&TrackList> {
        self.lists.get(uri)
    }

    pub fn has(&self, uri: &str) -> bool {
        self.lists.contains_key(uri)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.lists.len()
    }

    /// Every row of every list kept.
    pub fn all_rows(&self) -> impl Iterator<Item = &TrackRow> {
        self.lists.values().flat_map(|l| l.rows().iter())
    }

    /// Change a list's rows; when the list is `shown`, its view keeps its selected row when the edit ends.
    pub fn edit(&mut self, uri: &str, shown: bool) -> Option<Edit<'_>> {
        if !self.lists.contains_key(uri) {
            return None;
        }
        let before = if shown { self.anchor(uri) } else { None };
        Some(Edit { open: self, uri: uri.to_string(), before })
    }

    /// A page from the Daemon (the list is made when its first page comes).
    pub fn add_page(&mut self, p: &TrackPage, shown: bool) {
        self.lists.entry(p.uri.clone()).or_default();
        if let Some(mut l) = self.edit(&p.uri, shown) {
            l.add(p);
        }
    }

    /// A list's header, to change (its rows stay as they are).
    pub fn header_mut(&mut self, uri: &str) -> Option<&mut ListHeader> {
        self.lists.get_mut(uri)?.header.as_mut()
    }

    /// Let a list go, to be fetched again; its view stays.
    pub fn drop_list(&mut self, uri: &str) {
        self.lists.remove(uri);
    }

    /// Let a list and its view go.
    pub fn forget(&mut self, uri: &str) {
        self.lists.remove(uri);
        self.views.remove(uri);
    }

    // ── the views ──

    pub fn view(&self, uri: &str) -> Option<&TrackView> {
        self.views.get(uri)
    }

    /// A list's view, for moving the selection, scrolling, sorting or filtering.
    pub fn view_mut(&mut self, uri: &str) -> Option<&mut TrackView> {
        self.views.get_mut(uri)
    }

    /// Open `uri`'s list: as it was left, else as `fresh` makes it.
    pub fn open(&mut self, uri: &str, fresh: impl FnOnce() -> TrackView) -> &mut TrackView {
        self.views.entry(uri.to_string()).or_insert_with(fresh)
    }

    /// Take a view out while it's drawn (drawing needs the App too), and put it back.
    pub(crate) fn take_view(&mut self, uri: &str) -> Option<TrackView> {
        self.views.remove(uri)
    }

    pub(crate) fn put_view(&mut self, v: TrackView) {
        self.views.insert(v.uri.clone(), v);
    }

    /// What `uri`'s list is filtered by. The selected row stays selected, on its line, while it still shows;
    /// once the filter hides it, the row that took its place is.
    pub fn set_filter(&mut self, uri: &str, filter: String) {
        let before = self.anchor(uri);
        let Some(v) = self.views.get_mut(uri) else { return };
        v.filter = filter;
        if before.is_none() {
            (v.sel, v.top) = (0, 0);
        }
        self.settle(uri, before);
    }

    // ── rows of a list as its view shows them ──

    /// `uri`'s rows in display order, as indexes into its rows.
    pub fn order(&self, uri: &str) -> Rc<[usize]> {
        match self.views.get(uri).and_then(|v| Some((v, self.lists.get(uri)?))) {
            Some((v, l)) => v.order(l),
            None => Rc::new([]),
        }
    }

    /// The row shown at view position `vi` of `uri`'s list, and its index in the list.
    pub fn row_at(&self, uri: &str, vi: usize) -> Option<(usize, &TrackRow)> {
        let v = self.views.get(uri)?;
        let l = self.lists.get(uri)?;
        let ri = *v.order(l).get(vi)?;
        Some((ri, l.rows().get(ri)?))
    }

    pub fn id_at(&self, uri: &str, vi: usize) -> Option<RowId> {
        self.row_at(uri, vi).map(|(_, r)| RowId::of(r))
    }

    /// Where the row `id` shows now in `uri`'s list, if it does.
    pub fn find(&self, uri: &str, id: &RowId) -> Option<(usize, &TrackRow)> {
        let v = self.views.get(uri)?;
        let l = self.lists.get(uri)?;
        let rows = l.rows();
        v.order(l).iter().enumerate().find(|(_, ri)| RowId::of(&rows[**ri]) == *id).map(|(vi, ri)| (vi, &rows[*ri]))
    }

    /// The selected row of `uri`'s view and its line on screen, and the first row shown when the wheel has the view.
    fn anchor(&self, uri: &str) -> Option<Anchor> {
        let v = self.views.get(uri)?;
        let top = if v.wheeled.holds(v.sel) { self.id_at(uri, v.top) } else { None };
        Some(Anchor { sel: self.id_at(uri, v.sel)?, line: v.sel.saturating_sub(v.top), top })
    }

    /// After an edit: the selection back on its row, on the same line; else on the row that took its place. A view the
    /// wheel has keeps its first row instead (D86: pages arriving while a long list loads don't pull it back).
    fn settle(&mut self, uri: &str, before: Option<Anchor>) {
        let Some(a) = before else { return };
        let found = self.find(uri, &a.sel).map(|(vi, _)| vi);
        let top = a.top.and_then(|id| self.find(uri, &id)).map(|(vi, _)| vi);
        let len = self.order(uri).len();
        let Some(v) = self.views.get_mut(uri) else { return };
        match (found, top) {
            (Some(vi), Some(t)) => {
                (v.sel, v.top) = (vi, t);
                v.wheeled.keep(vi);
            }
            (Some(vi), None) => {
                v.sel = vi;
                v.top = vi.saturating_sub(a.line);
            }
            (None, _) => {
                v.sel = v.sel.min(len.saturating_sub(1));
                v.reveal();
            }
        }
    }
}

/// Where a view was before an edit: its selected row and that row's line on screen, and its first row while the wheel
/// has it.
pub struct Anchor {
    sel: RowId,
    line: usize,
    top: Option<RowId>,
}

/// A list being changed: its view, when shown, settles on its row again when this goes.
pub struct Edit<'a> {
    open: &'a mut OpenList,
    uri: String,
    before: Option<Anchor>,
}

impl std::ops::Deref for Edit<'_> {
    type Target = TrackList;
    fn deref(&self) -> &TrackList {
        &self.open.lists[&self.uri]
    }
}

impl std::ops::DerefMut for Edit<'_> {
    fn deref_mut(&mut self) -> &mut TrackList {
        self.open.lists.get_mut(&self.uri).expect("an edited list is kept")
    }
}

impl Drop for Edit<'_> {
    fn drop(&mut self) {
        let before = self.before.take();
        let uri = std::mem::take(&mut self.uri);
        self.open.settle(&uri, before);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracks::Col;

    fn row(name: &str, uid: &str) -> TrackRow {
        TrackRow { uri: format!("spotify:track:{name}"), name: name.into(), uid: Some(uid.into()), playable: true, ..Default::default() }
    }

    const P: &str = "spotify:playlist:p";

    fn page(offset: u32, total: u32, rows: Vec<TrackRow>) -> TrackPage {
        TrackPage { uri: "spotify:playlist:p".into(), offset, total, tracks: rows, ..Default::default() }
    }

    /// "spotify:playlist:p" open, sorted by title, with `names` arrived of `total`.
    fn open_sorted(names: &[&str], total: u32) -> OpenList {
        let mut o = OpenList::default();
        o.add_page(&page(0, total, names.iter().enumerate().map(|(i, n)| row(n, &format!("u{i}"))).collect()), true);
        o.open("spotify:playlist:p", || TrackView::new("spotify:playlist:p", "P"));
        o.view_mut(P).unwrap().cycle_sort(Col::Title);
        o
    }

    fn selected(o: &OpenList) -> String {
        o.row_at(P, o.view(P).unwrap().sel).unwrap().1.name.clone()
    }

    #[test]
    fn a_page_arriving_into_a_sorted_list_keeps_the_selected_row_on_its_line() {
        let mut o = open_sorted(&["d", "b", "f"], 6);
        // shown b d f; select d, scrolled so it's on the first line
        let v = o.view_mut(P).unwrap();
        v.sel = 1;
        v.top = 1;
        o.add_page(&page(3, 6, vec![row("a", "u3"), row("c", "u4"), row("e", "u5")]), true);
        // shown a b c d e f: d moved from 1 to 3
        assert_eq!(selected(&o), "d", "the same song stays selected");
        let v = o.view(P).unwrap();
        assert_eq!((v.sel, v.sel - v.top), (3, 0), "on the same line of the screen");
    }

    #[test]
    fn when_the_selected_row_goes_the_one_that_took_its_place_is_selected() {
        let mut o = open_sorted(&["a", "b", "c"], 3);
        o.view_mut(P).unwrap().sel = 1;
        o.edit(P, true).unwrap().take_uid("u1");
        assert_eq!(selected(&o), "c");
        o.edit(P, true).unwrap().take_uid("u2");
        assert_eq!(selected(&o), "a", "the last row went: the one now last");
    }

    #[test]
    fn a_filter_keeps_the_selected_row_while_it_shows_and_then_the_one_in_its_place() {
        let mut o = open_sorted(&["alpha", "beta", "gamma", "delta"], 4);
        // shown alpha beta delta gamma; select delta
        o.view_mut(P).unwrap().sel = 2;
        o.set_filter(P, "a".into()); // all four still show
        assert_eq!(selected(&o), "delta");
        o.set_filter(P, "ta".into()); // beta delta
        assert_eq!((selected(&o), o.view(P).unwrap().sel), ("delta".into(), 1));
        o.set_filter(P, "bet".into()); // beta only: delta is hidden, beta took its place
        assert_eq!(selected(&o), "beta");
        o.set_filter(P, String::new());
        assert_eq!(selected(&o), "beta", "and stays on it when the filter goes");
    }

    #[test]
    fn a_row_is_found_by_identity_wherever_it_shows() {
        let mut o = open_sorted(&["b", "a"], 4);
        let id = o.id_at(P, 0).unwrap();
        assert_eq!(o.find(P, &id).map(|(vi, r)| (vi, r.name.clone())), Some((0, "a".into())));
        o.view_mut(P).unwrap().cycle_sort(Col::Title); // descending: b a
        assert_eq!(o.find(P, &id).map(|(vi, _)| vi), Some(1));
        o.edit(P, true).unwrap().take_uid("u1");
        assert_eq!(o.find(P, &id), None, "gone");
    }

    #[test]
    fn a_list_left_is_found_as_it_was_on_coming_back() {
        let mut o = open_sorted(&["a", "b"], 2);
        o.view_mut(P).unwrap().sel = 1;
        o.open("spotify:album:x", || TrackView::new("spotify:album:x", "X"));
        o.open(P, || unreachable!("remembered"));
        assert_eq!(o.view(P).unwrap().sel, 1);
    }
}
