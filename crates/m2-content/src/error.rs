//! What is wrong with a content folder, with the file, line and column.

use std::fmt;
use std::ops::Range;

/// At most this many errors are collected; past it, one more says so.
pub const MAX_ERRORS: usize = 1_000;

/// One problem in a content folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentError {
    /// The file, relative to the folder (`spawns/a1.toml`).
    pub file: String,
    /// The line (from 1), when the problem has a place in the file.
    pub line: Option<usize>,
    /// The column (from 1, in characters), with the line.
    pub column: Option<usize>,
    /// What is wrong.
    pub message: String,
}

impl ContentError {
    /// A problem with a whole file.
    pub fn file(file: &str, message: impl Into<String>) -> Self {
        Self {
            file: file.to_owned(),
            line: None,
            column: None,
            message: message.into(),
        }
    }

    /// A problem at byte `span.start` of `text`.
    pub(crate) fn at(
        file: &str,
        text: Option<&str>,
        span: Option<Range<usize>>,
        message: impl Into<String>,
    ) -> Self {
        let place = text
            .zip(span)
            .map(|(text, span)| line_and_column(text, span.start));
        Self {
            file: file.to_owned(),
            line: place.map(|(line, _)| line),
            column: place.map(|(_, column)| column),
            message: message.into(),
        }
    }
}

impl fmt::Display for ContentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.line, self.column) {
            (Some(line), Some(column)) => {
                write!(f, "{}:{line}:{column}: {}", self.file, self.message)
            }
            (Some(line), None) => write!(f, "{}:{line}: {}", self.file, self.message),
            _ => write!(f, "{}: {}", self.file, self.message),
        }
    }
}

impl std::error::Error for ContentError {}

/// Every problem found in a content folder, in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Errors(Vec<ContentError>);

impl Errors {
    /// Adds a problem, up to [`MAX_ERRORS`].
    pub(crate) fn push(&mut self, error: ContentError) {
        match self.0.len().cmp(&MAX_ERRORS) {
            std::cmp::Ordering::Less => self.0.push(error),
            std::cmp::Ordering::Equal => self.0.push(ContentError::file(
                &error.file,
                format!("more than {MAX_ERRORS} problems; the rest are not listed"),
            )),
            std::cmp::Ordering::Greater => {}
        }
    }

    /// No problems.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// How many problems.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// The problems, in order.
    pub fn iter(&self) -> impl Iterator<Item = &ContentError> {
        self.0.iter()
    }

    pub(crate) fn into_result<T>(self, value: T) -> Result<T, Errors> {
        if self.0.is_empty() {
            Ok(value)
        } else {
            Err(self)
        }
    }
}

impl From<ContentError> for Errors {
    fn from(error: ContentError) -> Self {
        Self(vec![error])
    }
}

impl FromIterator<ContentError> for Errors {
    fn from_iter<I: IntoIterator<Item = ContentError>>(errors: I) -> Self {
        let mut all = Errors::default();
        for error in errors {
            all.push(error);
        }
        all
    }
}

impl fmt::Display for Errors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, error) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Errors {}

/// The line and column (both from 1; the column in characters) of byte
/// `offset` in `text`.
fn line_and_column(text: &str, offset: usize) -> (usize, usize) {
    let mut end = offset.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let before = text.get(..end).unwrap_or_default();
    let line = before.matches('\n').count() + 1;
    let start = before.rfind('\n').map_or(0, |at| at + 1);
    let column = before.get(start..).map_or(0, |rest| rest.chars().count()) + 1;
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_columns_count_from_one() {
        let text = "a = 1\nbé = 2\n";
        assert_eq!(line_and_column(text, 0), (1, 1));
        assert_eq!(line_and_column(text, 6), (2, 1));
        // After the two bytes of é: the third character.
        assert_eq!(line_and_column(text, 9), (2, 3));
        assert_eq!(line_and_column(text, 999), (3, 1));
    }
}
