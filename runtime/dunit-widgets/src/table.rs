//! Immutable typed table data, stable ordering and shared column geometry.
use alloc::{
    collections::BTreeSet,
    rc::Rc,
    string::{String, ToString},
    vec::Vec,
};
use core::cmp::Ordering;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableValue {
    Text(String),
    Integer(i64),
    Unsigned(u64),
}
impl TableValue {
    pub fn text(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::Integer(n) => n.to_string(),
            Self::Unsigned(n) => n.to_string(),
        }
    }
    fn compare(&self, b: &Self) -> Ordering {
        match (self, b) {
            (Self::Text(a), Self::Text(b)) => a.cmp(b),
            (Self::Text(_), _) => Ordering::Greater,
            (_, Self::Text(_)) => Ordering::Less,
            (a, b) => {
                let number = |n: &Self| match n {
                    Self::Integer(n) => *n as i128,
                    Self::Unsigned(n) => *n as i128,
                    Self::Text(_) => unreachable!(),
                };
                number(a).cmp(&number(b))
            }
        }
    }
}
impl From<&str> for TableValue {
    fn from(v: &str) -> Self {
        Self::Text(v.into())
    }
}
impl From<String> for TableValue {
    fn from(v: String) -> Self {
        Self::Text(v)
    }
}
impl From<i64> for TableValue {
    fn from(v: i64) -> Self {
        Self::Integer(v)
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct TableColumn {
    pub key: String,
    pub label: String,
    pub width: Option<f32>,
    pub sortable: bool,
    pub editable: bool,
}
impl TableColumn {
    pub fn new(key: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            width: None,
            sortable: true,
            editable: false,
        }
    }
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }
    pub fn sortable(mut self, v: bool) -> Self {
        self.sortable = v;
        self
    }
    pub fn editable(mut self, v: bool) -> Self {
        self.editable = v;
        self
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRow {
    pub key: String,
    pub cells: Vec<TableValue>,
    pub disabled: bool,
}
impl TableRow {
    pub fn new(key: impl Into<String>, cells: Vec<TableValue>) -> Self {
        Self {
            key: key.into(),
            cells,
            disabled: false,
        }
    }
    pub fn disabled(mut self, v: bool) -> Self {
        self.disabled = v;
        self
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Ascending,
    Descending,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSort {
    pub column: String,
    pub order: SortOrder,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableError {
    NoColumns,
    InvalidColumn,
    DuplicateColumn,
    InvalidRow,
    DuplicateRow,
    CellCount,
}
#[derive(Debug, Clone, PartialEq)]
struct Data {
    columns: Vec<TableColumn>,
    rows: Vec<TableRow>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct TableModel(Rc<Data>);
impl TableModel {
    pub fn new(columns: Vec<TableColumn>, rows: Vec<TableRow>) -> Result<Self, TableError> {
        if columns.is_empty() {
            return Err(TableError::NoColumns);
        }
        let mut keys = BTreeSet::new();
        for c in &columns {
            if c.key.is_empty() || c.width.is_some_and(|w| !w.is_finite() || w <= 0.) {
                return Err(TableError::InvalidColumn);
            }
            if !keys.insert(&c.key) {
                return Err(TableError::DuplicateColumn);
            }
        }
        keys.clear();
        for r in &rows {
            if r.key.is_empty() {
                return Err(TableError::InvalidRow);
            }
            if !keys.insert(&r.key) {
                return Err(TableError::DuplicateRow);
            }
            if r.cells.len() != columns.len() {
                return Err(TableError::CellCount);
            }
        }
        Ok(Self(Rc::new(Data { columns, rows })))
    }
    pub fn columns(&self) -> &[TableColumn] {
        &self.0.columns
    }
    pub fn rows(&self) -> &[TableRow] {
        &self.0.rows
    }
    pub fn column(&self, key: &str) -> Option<usize> {
        self.0.columns.iter().position(|c| c.key == key)
    }
    pub fn order(&self, sort: Option<&TableSort>) -> Option<Vec<usize>> {
        let mut order: Vec<_> = (0..self.0.rows.len()).collect();
        if let Some(sort) = sort {
            let column = self
                .column(&sort.column)
                .filter(|&i| self.0.columns[i].sortable)?;
            order.sort_by(|&a, &b| {
                let cmp = self.0.rows[a].cells[column].compare(&self.0.rows[b].cells[column]);
                match sort.order {
                    SortOrder::Ascending => cmp,
                    SortOrder::Descending => cmp.reverse(),
                }
            });
        }
        Some(order)
    }
    pub fn widths(&self, default: f32) -> Option<Vec<f32>> {
        if !default.is_finite() || default <= 0. {
            return None;
        }
        let widths: Vec<_> = self
            .columns()
            .iter()
            .map(|c| c.width.unwrap_or(default))
            .collect();
        widths
            .iter()
            .try_fold(0f32, |a, b| (a + b).is_finite().then_some(a + b))?;
        Some(widths)
    }
}
/// Resolve exactly the cell rectangles used by table painting and input.
pub fn column_rect(
    viewport: dunit_ui::Rect,
    widths: &[f32],
    offset: f32,
    column: usize,
) -> Option<dunit_ui::Rect> {
    let w = *widths.get(column)?;
    if !offset.is_finite() || offset < 0. || widths.iter().any(|w| !w.is_finite() || *w <= 0.) {
        return None;
    }
    let x = viewport.x + widths[..column].iter().sum::<f32>() - offset;
    (x.is_finite() && w.is_finite()).then_some(dunit_ui::Rect { x, w, ..viewport })
}
pub fn column_at(
    viewport: dunit_ui::Rect,
    widths: &[f32],
    offset: f32,
    x: f32,
    y: f32,
) -> Option<usize> {
    if !x.is_finite()
        || !y.is_finite()
        || x < viewport.x
        || x >= viewport.x + viewport.w
        || y < viewport.y
        || y >= viewport.y + viewport.h
    {
        return None;
    }
    widths.iter().enumerate().find_map(|(i, _)| {
        column_rect(viewport, widths, offset, i)
            .filter(|r| x >= r.x && x < r.x + r.w)
            .map(|_| i)
    })
}

impl From<u64> for TableValue {
    fn from(v: u64) -> Self {
        Self::Unsigned(v)
    }
}
