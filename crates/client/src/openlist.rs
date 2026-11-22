//! The open Track list (D81): the Track lists received so far, the one open in the main Pane with its sort, filter,
//! selection and scroll, and the views remembered per list so going back finds a list as you left it.
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
    view: Option<TrackView>,
    remembered: HashMap<String, TrackView>,
}

impl OpenList {
    // ── the lists ──

    pub fn list(&self, uri: &str) -> Option<&TrackList> {
        self.lists.get(uri)
    }

    pub fn has(&self, uri: &str) -> bool {
        self.lists.contains_key(uri)
    }

    pub fn len(&self) -> usize {
        self.lists.len()
    }

    /// Every row of every list kept.
    pub fn all_rows(&self) -> impl Iterator<Item = &TrackRow> {
        self.lists.values().flat_map(|l| l.rows().iter())
    }

    /// Change a list's rows; the open view keeps its selected row when the edit ends.
    pub fn edit(&mut self, uri: &str) -> Option<Edit<'_>> {
        if !self.lists.contains_key(uri) {
            return None;
        }
        let before = self.anchor(uri);
        Some(Edit { open: self, uri: uri.to_string(), before })
    }

    /// A page from the Daemon (the list is made when its first page comes).
    pub fn add_page(&mut self, p: &TrackPage) {
        self.lists.entry(p.uri.clone()).or_default();
        if let Some(mut l) = self.edit(&p.uri) {
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

    /// Let a list and its remembered view go.
    pub fn forget(&mut self, uri: &str) {
        self.lists.remove(uri);
        self.remembered.remove(uri);
    }

    // ── the open view ──

    pub fn view(&self) -> Option<&TrackView> {
        self.view.as_ref()
    }

    /// The open view, for moving the selection, scrolling, sorting or filtering.
    pub fn view_mut(&mut self) -> Option<&mut TrackView> {
        self.view.as_mut()
    }

    /// Is `uri` the list open now?
    pub fn is_open(&self, uri: &str) -> bool {
        self.view.as_ref().is_some_and(|v| v.uri == uri)
    }

    /// Open `uri`'s list: as it was left, else as `fresh` makes it. The list open before is remembered.
    pub fn open(&mut self, uri: &str, fresh: impl FnOnce() -> TrackView) {
        self.close();
        self.view = Some(self.remembered.remove(uri).unwrap_or_else(fresh));
    }

    /// Close the open list, remembering how it was looked at.
    pub fn close(&mut self) {
        if let Some(v) = self.view.take() {
            self.remembered.insert(v.uri.clone(), v);
        }
    }

    /// A view remembered for `uri` (a list not open now).
    pub fn remembered_mut(&mut self, uri: &str) -> Option<&mut TrackView> {
        self.remembered.get_mut(uri)
    }

    /// Take the open view out while it's drawn (drawing needs the App too), and put it back.
    pub(crate) fn take_view(&mut self) -> Option<TrackView> {
        self.view.take()
    }

    pub(crate) fn put_view(&mut self, v: TrackView) {
        self.view = Some(v);
    }

    // ── rows of the open list ──

    /// The open list's rows in display order, as indexes into its rows.
    pub fn order(&self) -> Rc<[usize]> {
        match self.view.as_ref().and_then(|v| Some((v, self.lists.get(&v.uri)?))) {
            Some((v, l)) => v.order(l),
            None => Rc::new([]),
        }
    }

    /// The row shown at view position `vi`, and its index in the list.
    pub fn row_at(&self, vi: usize) -> Option<(usize, &TrackRow)> {
        let v = self.view.as_ref()?;
        let l = self.lists.get(&v.uri)?;
        let ri = *v.order(l).get(vi)?;
        Some((ri, l.rows().get(ri)?))
    }

    pub fn id_at(&self, vi: usize) -> Option<RowId> {
        self.row_at(vi).map(|(_, r)| RowId::of(r))
    }

    /// Where the row `id` shows now, if it does.
    pub fn find(&self, id: &RowId) -> Option<(usize, &TrackRow)> {
        let v = self.view.as_ref()?;
        let l = self.lists.get(&v.uri)?;
        let rows = l.rows();
        v.order(l).iter().enumerate().find(|(_, ri)| RowId::of(&rows[**ri]) == *id).map(|(vi, ri)| (vi, &rows[*ri]))
    }

    /// The selected row and its line on screen, when `uri` is open.
    fn anchor(&self, uri: &str) -> Option<(RowId, usize)> {
        let v = self.view.as_ref().filter(|v| v.uri == uri)?;
        Some((self.id_at(v.sel)?, v.sel.saturating_sub(v.top)))
    }

    /// After an edit: the selection back on its row, on the same line; else on the row that took its place.
    fn settle(&mut self, before: Option<(RowId, usize)>) {
        let Some((id, line)) = before else { return };
        let found = self.find(&id).map(|(vi, _)| vi);
        let len = self.order().len();
        let Some(v) = self.view.as_mut() else { return };
        match found {
            Some(vi) => {
                v.sel = vi;
                v.top = vi.saturating_sub(line);
            }
            None => {
                v.sel = v.sel.min(len.saturating_sub(1));
                v.reveal();
            }
        }
    }
}

/// A list being changed: the open view settles on its row again when this goes.
pub struct Edit<'a> {
    open: &'a mut OpenList,
    uri: String,
    before: Option<(RowId, usize)>,
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
        self.open.settle(before);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracks::Col;

    fn row(name: &str, uid: &str) -> TrackRow {
        TrackRow { uri: format!("spotify:track:{name}"), name: name.into(), uid: Some(uid.into()), playable: true, ..Default::default() }
    }

    fn page(offset: u32, total: u32, rows: Vec<TrackRow>) -> TrackPage {
        TrackPage { uri: "spotify:playlist:p".into(), offset, total, tracks: rows, ..Default::default() }
    }

    /// "spotify:playlist:p" open, sorted by title, with `names` arrived of `total`.
    fn open_sorted(names: &[&str], total: u32) -> OpenList {
        let mut o = OpenList::default();
        o.add_page(&page(0, total, names.iter().enumerate().map(|(i, n)| row(n, &format!("u{i}"))).collect()));
        o.open("spotify:playlist:p", || TrackView::new("spotify:playlist:p", "P"));
        o.view_mut().unwrap().cycle_sort(Col::Title);
        o
    }

    fn selected(o: &OpenList) -> String {
        o.row_at(o.view().unwrap().sel).unwrap().1.name.clone()
    }

    #[test]
    fn a_page_arriving_into_a_sorted_list_keeps_the_selected_row_on_its_line() {
        let mut o = open_sorted(&["d", "b", "f"], 6);
        // shown b d f; select d, scrolled so it's on the first line
        let v = o.view_mut().unwrap();
        v.sel = 1;
        v.top = 1;
        o.add_page(&page(3, 6, vec![row("a", "u3"), row("c", "u4"), row("e", "u5")]));
        // shown a b c d e f: d moved from 1 to 3
        assert_eq!(selected(&o), "d", "the same song stays selected");
        let v = o.view().unwrap();
        assert_eq!((v.sel, v.sel - v.top), (3, 0), "on the same line of the screen");
    }

    #[test]
    fn when_the_selected_row_goes_the_one_that_took_its_place_is_selected() {
        let mut o = open_sorted(&["a", "b", "c"], 3);
        o.view_mut().unwrap().sel = 1;
        o.edit("spotify:playlist:p").unwrap().take_uid("u1");
        assert_eq!(selected(&o), "c");
        o.edit("spotify:playlist:p").unwrap().take_uid("u2");
        assert_eq!(selected(&o), "a", "the last row went: the one now last");
    }

    #[test]
    fn a_row_is_found_by_identity_wherever_it_shows() {
        let mut o = open_sorted(&["b", "a"], 4);
        let id = o.id_at(0).unwrap();
        assert_eq!(o.find(&id).map(|(vi, r)| (vi, r.name.clone())), Some((0, "a".into())));
        o.view_mut().unwrap().cycle_sort(Col::Title); // descending: b a
        assert_eq!(o.find(&id).map(|(vi, _)| vi), Some(1));
        o.edit("spotify:playlist:p").unwrap().take_uid("u1");
        assert_eq!(o.find(&id), None, "gone");
    }

    #[test]
    fn a_list_left_is_found_as_it_was_on_coming_back() {
        let mut o = open_sorted(&["a", "b"], 2);
        o.view_mut().unwrap().sel = 1;
        o.open("spotify:album:x", || TrackView::new("spotify:album:x", "X"));
        assert!(o.is_open("spotify:album:x"));
        o.open("spotify:playlist:p", || unreachable!("remembered"));
        assert_eq!(o.view().unwrap().sel, 1);
    }
}
