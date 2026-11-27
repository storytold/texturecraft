//! What a comparison's rows picked

/// What a comparison's rows picked: a source per side (left, right) and the operator
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CompareRowResponse<T> {
    pub(crate) left: Option<T>,
    pub(crate) right: Option<T>,
    pub(crate) operator: Option<usize>,
}
