/// The text a row is editing in place
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowEdit {
    Name,
    Description,
}
