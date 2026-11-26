/// What the bitmap picker offers: Import..., an image resource, a library alpha
#[derive(Clone, PartialEq)]
pub(super) enum BitmapChoice {
    Import,
    Image(String),
    Alpha(&'static str),
}
