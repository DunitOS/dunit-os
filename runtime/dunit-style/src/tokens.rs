//! Queryable values from the same DSS definitions used during expansion.
use crate::{Color, Value};
use alloc::{collections::BTreeMap, string::String, vec::Vec};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tokens {
    pub(crate) words: BTreeMap<String, Vec<String>>,
}
impl Tokens {
    /// Names exclude the leading `$`; missing names are never defaulted.
    pub fn get(&self, name: &str) -> Option<Value> {
        self.words
            .get(name)
            .map(|words| crate::parse::words_to_value(words))
    }
    pub fn color(&self, name: &str) -> Option<Color> {
        self.get(name)?.as_color()
    }
    pub fn number(&self, name: &str) -> Option<f32> {
        self.get(name)?.as_number()
    }
    pub fn duration(&self, name: &str) -> Option<f32> {
        self.get(name)?.as_duration()
    }
    pub fn iter(&self) -> impl Iterator<Item = (&str, Value)> {
        self.words
            .iter()
            .map(|(name, words)| (name.as_str(), crate::parse::words_to_value(words)))
    }
}
