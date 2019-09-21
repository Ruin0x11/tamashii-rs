#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub filename: String,
    pub size: u64,
    pub ext: String,
    pub attrs: Vec<u32>,
}
