use core::panic;
use std::{borrow::Cow, fmt::Display, iter::Peekable, slice, str::Chars, sync::Arc};

use percent_encoding::{percent_encode, AsciiSet, CONTROLS, NON_ALPHANUMERIC};
use regex::{CaptureLocations, CaptureNames, Regex};

// --------------------------------------------------

mod deserializer;

pub(crate) use deserializer::{DeserializerError, FromParamsList};

use crate::common::SCOPE_VALIDITY;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------

pub(crate) fn split_uri_host_and_path(uri: &str) -> (Option<&str>, Option<&str>) {
	if uri.is_empty() {
		panic!("empty URI")
	}

	if let Some(uri) = uri
		.strip_prefix("https://")
		.or_else(|| uri.strip_prefix("http://"))
	{
		if let Some(position) = uri.find("/") {
			if position == 0 {
				return (None, Some(uri));
			}

			return (Some(&uri[..position]), Some(&uri[position..]));
		}

		return (Some(uri), None);
	}

	(None, Some(uri))
}

// --------------------------------------------------
// static:		static, resource
// regex: {capture_name:pattern}escaped{capture_name}.escaped{capture_name:pattern}
// wildcard: {name}

#[derive(Debug, Clone)]
pub(crate) enum Pattern {
	Static(Arc<str>),
	Regex(RegexNames, Regex),
	Wildcard(Arc<str>),
}

// ???
const ASCII_SET: &AsciiSet = &CONTROLS
	.add(b' ')
	.add(b'%')
	.add(b'/')
	.add(b'?')
	.add(b'#')
	.add(b'[')
	.add(b']')
	.add(b'<')
	.add(b'>')
	.add(b'\\')
	.add(b'^')
	.add(b':');

impl Pattern {
	pub(crate) fn parse(pattern: &str) -> Pattern {
		let mut chars = pattern.chars().peekable();

		let mut segments = split(chars);

		if segments.is_empty() {
			panic!("incomplete regex pattern")
		}

		let replacer = Regex::new("(%2f)|(%2F)").expect("hard coded regex replacer must be valid");

		if segments.len() == 1 {
			match segments.pop().expect(SCOPE_VALIDITY) {
				Segment::Static(static_pattern) => {
					if static_pattern == "/" {
						return Pattern::Static(pattern.into());
					}

					let static_pattern = replacer.replace_all(&static_pattern, "/");

					let encoded_static_pattern =
						Cow::<str>::from(percent_encode(static_pattern.as_bytes(), ASCII_SET));

					return Pattern::Static(encoded_static_pattern.into());
				}
				Segment::Capturing {
					name: capture_name,
					some_subpattern,
				} => {
					let Some(subpattern) = some_subpattern else {
						return Pattern::Wildcard((*capture_name).into());
					};

					let subpattern = replacer.replace_all(&subpattern, "/");

					let regex_subpattern = format!(r"\A(?P<{}>{})\z", capture_name, subpattern);
					match Regex::new(&regex_subpattern) {
						Ok(regex) => {
							let capture_names = regex.capture_names();

							return Pattern::Regex(RegexNames::new(capture_names), regex);
						}
						Err(error) => panic!("{}", error),
					}
				}
			};
		}

		let mut regex_pattern = "\\A".to_owned();

		let end_index = segments.len() - 1;

		for (index, segment) in segments.into_iter().enumerate() {
			match segment {
				Segment::Static(static_pattern) => {
					let static_pattern = regex::escape(replacer.replace_all(&static_pattern, "/").as_ref());
					regex_pattern.push_str(&static_pattern);
				}
				Segment::Capturing {
					name: capture_name,
					some_subpattern,
				} => {
					let subpattern = if let Some(subpattern) = some_subpattern.as_ref() {
						replacer.replace_all(&subpattern, "/")
					} else {
						if index == end_index {
							Cow::Borrowed(".+")
						} else {
							Cow::Borrowed("[^.]+")
						}
					};

					regex_pattern.push_str(&format!("(?P<{}>{})", &capture_name, subpattern));
				}
			}
		}

		regex_pattern.push_str("\\z");
		match Regex::new(&regex_pattern) {
			Ok(regex) => {
				let capture_names = regex.capture_names();

				Pattern::Regex(RegexNames::new(capture_names), regex)
			}
			Err(error) => panic!("{}", error),
		}
	}

	#[inline(always)]
	pub(crate) fn is_static(&self) -> bool {
		if let Pattern::Static(_) = self {
			return true;
		}

		false
	}

	#[inline(always)]
	pub(crate) fn is_regex(&self) -> bool {
		if let Pattern::Regex(_, _) = self {
			return true;
		}

		false
	}

	#[inline(always)]
	pub(crate) fn is_wildcard(&self) -> bool {
		if let Pattern::Wildcard(_) = self {
			return true;
		}

		false
	}

	#[inline(always)]
	pub(crate) fn is_static_match(&self, text: &str) -> Option<bool> {
		if let Self::Static(pattern) = self {
			if pattern.as_ref() == text {
				return Some(true);
			}

			return Some(false);
		}

		None
	}

	#[inline]
	pub(crate) fn is_regex_match(&self, text: &str, params_list: &mut ParamsList) -> Option<bool> {
		if let Self::Regex(capture_names, regex) = self {
			let mut capture_locations = regex.capture_locations();
			if regex.captures_read(&mut capture_locations, text).is_some() {
				params_list.push(Params::with_regex_captures(
					capture_names.clone(),
					capture_locations,
					text.into(),
				));

				return Some(true);
			}

			return Some(false);
		}

		None
	}

	#[inline(always)]
	pub(crate) fn is_wildcard_match(
		&self,
		text: Cow<str>,
		params_list: &mut ParamsList,
	) -> Option<bool> {
		if let Self::Wildcard(name) = self {
			params_list.push(Params::with_wildcard_value(name.clone(), text.into()));

			Some(true)
		} else {
			None
		}
	}

	pub(crate) fn compare(&self, other: &Self) -> Similarity {
		match self {
			Pattern::Static(pattern) => {
				if let Pattern::Static(other_pattern) = other {
					if pattern == other_pattern {
						return Similarity::Same;
					}
				}
			}
			Pattern::Regex(capture_names, regex) => {
				if let Pattern::Regex(other_capture_names, other_regex) = other {
					if regex.as_str() == other_regex.as_str() {
						return Similarity::Same;
					}

					return Similarity::Different;
				}
			}
			Pattern::Wildcard(name) => {
				if let Pattern::Wildcard(other_name) = other {
					if name == other_name {
						return Similarity::Same;
					}

					return Similarity::DifferentName;
				}
			}
		}

		Similarity::Different
	}
}

// -------------------------

impl Display for Pattern {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Pattern::Static(pattern) => write!(f, "static pattern: {}", pattern),
			Pattern::Regex(_, regex) => write!(f, "regex pattern: {}", regex),
			Pattern::Wildcard(name) => write!(f, "wildcard pattern: {}", name),
		}
	}
}

// -------------------------

impl Default for Pattern {
	fn default() -> Self {
		Pattern::Static("".into())
	}
}

// --------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RegexNames(Arc<[(Box<str>, usize)]>);

impl RegexNames {
	fn new(capture_names: CaptureNames) -> Self {
		let mut names = Vec::new();

		for (i, some_capture_name) in capture_names.enumerate() {
			if let Some(capture_name) = some_capture_name {
				names.push((Box::from(capture_name), i));
			}
		}

		Self(Arc::from(names))
	}

	#[inline]
	fn get(&self, index: usize) -> Option<(&str, usize)> {
		self.0.get(index).map(|elem| (elem.0.as_ref(), elem.1))
	}

	#[inline]
	pub(crate) fn has(&self, name: &str) -> bool {
		self
			.0
			.iter()
			.any(|(capture_name, _)| name == capture_name.as_ref())
	}

	#[inline]
	pub(crate) fn len(&self) -> usize {
		self.0.len()
	}
}

impl AsRef<[(Box<str>, usize)]> for RegexNames {
	fn as_ref(&self) -> &[(Box<str>, usize)] {
		&self.0
	}
}

// --------------------------------------------------

#[derive(Debug, Default, Clone)]
pub(crate) struct ParamsList(Vec<Params>);

impl ParamsList {
	#[inline]
	pub(crate) fn new() -> Self {
		ParamsList(Vec::new())
	}

	#[inline]
	fn push(&mut self, params: Params) {
		self.0.push(params)
	}

	#[inline]
	fn iter(&self) -> slice::Iter<'_, Params> {
		self.0.iter()
	}

	#[inline]
	pub(crate) fn deserializer(&mut self) -> FromParamsList<'_> {
		FromParamsList::new(&self.0)
	}
}

// --------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) enum Params {
	Regex(RegexNames, CaptureLocations, Box<str>),
	Wildcard(Arc<str>, Box<str>),
}

impl Params {
	#[inline]
	fn with_regex_captures(
		regex_names: RegexNames,
		capture_locations: CaptureLocations,
		values: Box<str>,
	) -> Self {
		Self::Regex(regex_names, capture_locations, values)
	}

	#[inline]
	fn with_wildcard_value(name: Arc<str>, value: Box<str>) -> Self {
		Self::Wildcard(name, value)
	}
}

impl ToString for Params {
	fn to_string(&self) -> String {
		let mut string = String::new();

		match self {
			Self::Regex(regex_names, capture_locations, values) => {
				string.push_str("regex params: [");

				let mut first = true;
				for (name, index) in regex_names.as_ref().iter() {
					let (start, end) = capture_locations
						.get(*index)
						.expect("capture name index in RegexNames must point to a valid capture location");

					let value = &values[start..end];

					if first {
						string.push_str(&format!("{}:{}", name, value));
						first = false
					} else {
						string.push_str(&format!(", {}:{}", name, value));
					}
				}

				string.push(']');
			}
			Self::Wildcard(name, value) => {
				string.push_str(&format!("wildcard param: [{}:{}]", name, value));
			}
		}

		string
	}
}

// --------------------------------------------------
// Parsing helpers

#[derive(PartialEq, Debug)]
enum Segment {
	Static(String),
	Capturing {
		name: String,
		some_subpattern: Option<String>,
	},
}

#[inline]
fn split(mut chars: Peekable<Chars>) -> Vec<Segment> {
	let mut slices = Vec::new();
	let mut parsing_static = true;

	loop {
		if parsing_static {
			let (static_segment, some_delimiter) = split_off_static_segment(&mut chars);
			if !static_segment.is_empty() {
				slices.push(Segment::Static(static_segment));
			}

			if some_delimiter.is_some() {
				parsing_static = false
			} else {
				break;
			}
		} else {
			let (name, some_delimiter) =
				split_at_delimiter(&mut chars, |ch| ch == ':' || ch == '}', |_| false);

			if name.is_empty() {
				panic!("empty regex capture name")
			}

			let Some(delimiter) = some_delimiter else {
				panic!("incomplete pattern")
			};

			if delimiter == '}' {
				if let Some(next_char) = chars.peek() {
					if *next_char != '.' {
						panic!(
							"a wildcard must be an only or the last segment, or it must be followed by a '.'",
						)
					}

					slices.push(Segment::Capturing {
						name,
						some_subpattern: None,
					});
				} else {
					slices.push(Segment::Capturing {
						name,
						some_subpattern: None,
					});

					break;
				}
			} else {
				let Some(subpattern) = split_off_subpattern(&mut chars) else {
					panic!("no closing brace of the regex subpattern was found")
				};

				if subpattern.is_empty() {
					panic!("empty regex subpattern")
				}

				slices.push(Segment::Capturing {
					name,
					some_subpattern: Some(subpattern),
				});
			}

			parsing_static = true;
		}
	}

	slices
}

// Returns the segment before the delimiter and the delimiter. If the delimiter is not
// found then the segment contains all the chars and the returned delimiter will be None.
// If there are no more chars or the delimiter is found right away then the returned
// segment will be empty.
fn split_at_delimiter(
	chars: &mut Peekable<Chars<'_>>,
	delimiter: impl Fn(char) -> bool,
	escaper: impl Fn(char) -> bool,
) -> (String, Option<char>) {
	let mut buf = String::new();
	let mut escaped = false;

	while let Some(ch) = chars.next() {
		if escaper(ch) {
			if !escaped {
				if let Some(next_ch) = chars.peek() {
					if delimiter(*next_ch) {
						escaped = true;

						continue;
					}
				}
			}
		}

		if delimiter(ch) {
			if !escaped {
				return (buf, Some(ch));
			}

			escaped = false;
		}

		buf.push(ch);
	}

	(buf, None)
}

fn split_off_static_segment(chars: &mut Peekable<Chars<'_>>) -> (String, Option<char>) {
	let mut buf = String::new();

	let mut escaped_opening_brace = false;
	let mut escaped_closing_brace = false;

	while let Some(ch) = chars.next() {
		if ch == '{' {
			if !escaped_opening_brace {
				if let Some(next_ch) = chars.peek() {
					if *next_ch == '{' {
						escaped_opening_brace = true;

						continue;
					}
				}

				return (buf, Some(ch));
			}

			escaped_opening_brace = false;
		} else if ch == '}' {
			if !escaped_closing_brace {
				if let Some(next_ch) = chars.peek() {
					if *next_ch == '}' {
						escaped_closing_brace = true;

						continue;
					}
				}
			}

			escaped_closing_brace = false;
		}

		buf.push(ch);
	}

	(buf, None)
}

// Returns the regex subpattern if the end of the regex segment is found. Otherwise None.
// The regex pattern may be empty if the end of the regex segment is met right away.
fn split_off_subpattern(chars: &mut Peekable<Chars<'_>>) -> Option<String> {
	let mut subpattern = String::new();
	let mut depth = 1; // We are already inside the opened '{' bracket.
	let mut unescaped = true;
	let mut in_character_class = -1i8;
	let mut in_named_capture_group = -1i8;

	while let Some(ch) = chars.next() {
		if ch == '}' && unescaped && in_character_class < 0 {
			depth -= 1;
			if depth == 0 {
				return Some(subpattern);
			}

			subpattern.push(ch);

			continue;
		}

		subpattern.push(ch);

		if in_character_class > -1 {
			in_character_class += 1;
		}

		match ch {
			'{' => {
				if unescaped || in_character_class < 0 {
					depth += 1;
				}
			}
			'\\' => {
				if unescaped {
					if let Some('\\' | '[' | ']' | '(' | ')' | '{' | '}') = chars.peek() {
						unescaped = false;

						continue;
					}
				}
			}
			'[' => {
				if unescaped || in_character_class < 0 {
					in_character_class = 0;
				}
			}
			']' => {
				if unescaped && in_character_class > -1 {
					in_character_class = -1;
				}
			}
			'^' => {
				if in_character_class == 1 {
					if let Some(']') = chars.peek() {
						unescaped = false;

						continue;
					}
				}
			}
			'(' => {
				if unescaped || in_character_class < 0 {
					if let Some('?') = chars.peek() {
						in_named_capture_group += 1;
					}
				}
			}
			')' => {
				if unescaped || in_character_class < 0 {
					if in_named_capture_group == 0 {
						in_named_capture_group -= 1;
					}
				}
			}
			'?' => {
				if in_named_capture_group == 0 {
					if let Some('P' | '<') = chars.peek() {
						in_named_capture_group += 1;
					} else {
						in_named_capture_group = -1;
					}
				}
			}
			'P' => {
				if in_named_capture_group == 1 {
					if let Some('<') = chars.peek() {
						panic!("regex subpattern cannot have a named capture group")
					}
				}
			}
			'<' => {
				if in_named_capture_group == 1 {
					if let Some('=' | '!') = chars.peek() {
						in_named_capture_group = -1;
					} else {
						panic!("regex subpattern cannot have a named capture group")
					}
				}
			}
			_ => {}
		}

		unescaped = true;
	}

	None
}

// --------------------------------------------------

#[repr(u8)]
#[derive(PartialEq, Debug)]
pub(crate) enum Similarity {
	Different,
	DifferentName,
	Same,
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(test)]
mod test {
	use super::*;

	// --------------------------------------------------
	// --------------------------------------------------

	#[test]
	fn split_at_delimiter() {
		let mut pattern = "escaped{capture:regex}".chars().peekable();

		let (escaped, some_delimiter) = super::split_at_delimiter(
			&mut pattern,
			|ch| ch == '{' || ch == ':' || ch == '}',
			|ch| ch == '}',
		);

		assert_eq!(escaped, "escaped");
		assert_eq!(some_delimiter, Some('{'));

		let (capture, some_delimiter) =
			super::split_at_delimiter(&mut pattern, |ch| ch == ':' || ch == '}', |_| false);

		assert_eq!(capture, "capture");
		assert_eq!(some_delimiter, Some(':'));

		let (regex, some_delimiter) = super::split_at_delimiter(
			&mut pattern,
			|ch| ch == '}' || ch == '{' || ch == ':',
			|_| false,
		);

		assert_eq!(regex, "regex");
		assert_eq!(some_delimiter, Some('}'));
	}

	#[test]
	fn split_off_subpattern() {
		let subpattern1 = r"(\d{4})-(\d{2})-(\d{2})";
		let subpattern2 = r"(.+)$";
		let subpattern3 = r"[^0-9)]+";
		let subpattern4 = r"[^])]";
		let subpattern5 = r"[^a]}]";
		let pattern = format!(
			"{{{}}}:{{{}}}:{{{}}}:{{{}}}:{{{}}}",
			subpattern1, subpattern2, subpattern3, subpattern4, subpattern5,
		);

		dbg!(&pattern);

		let mut pattern = pattern.chars().peekable();
		pattern.next(); // We must remove the opening braces.

		let subpattern = super::split_off_subpattern(&mut pattern);
		assert_eq!(subpattern, Some(subpattern1.to_owned()));
		println!("subpattern 1: {}", subpattern.unwrap());

		assert_eq!(pattern.next(), Some(':'));
		assert_eq!(pattern.next(), Some('{'));

		let subpattern = super::split_off_subpattern(&mut pattern);
		assert_eq!(subpattern, Some(subpattern2.to_owned()));
		println!("subpattern 2: {}", subpattern.unwrap());

		assert_eq!(pattern.next(), Some(':'));
		assert_eq!(pattern.next(), Some('{'));

		let subpattern = super::split_off_subpattern(&mut pattern);
		assert_eq!(subpattern, Some(subpattern3.to_owned()));
		println!("subpattern 3: {}", subpattern.unwrap());

		assert_eq!(pattern.next(), Some(':'));
		assert_eq!(pattern.next(), Some('{'));

		let subpattern = super::split_off_subpattern(&mut pattern);
		assert_eq!(subpattern, Some(subpattern4.to_owned()));
		println!("subpattern 4: {}", subpattern.unwrap());

		assert_eq!(pattern.next(), Some(':'));
		assert_eq!(pattern.next(), Some('{'));

		let subpattern = super::split_off_subpattern(&mut pattern);
		assert_ne!(subpattern, Some(subpattern5.to_owned()));
		println!("subpattern 5: {}", subpattern.unwrap());

		assert_eq!(pattern.next(), Some(']'));
	}

	#[test]
	fn split() {
		let cases = [
			(
				"static{capture_name:pattern}-{capture_name}",
				vec![
					Segment::Static("static".to_owned()),
					Segment::Capturing {
						name: "capture_name".to_owned(),
						some_subpattern: Some("pattern".to_owned()),
					},
					Segment::Static("-".to_owned()),
					Segment::Capturing {
						name: "capture_name".to_owned(),
						some_subpattern: None,
					},
				],
			),
			(
				"static{capture_name}.static",
				vec![
					Segment::Static("static".to_owned()),
					Segment::Capturing {
						name: "capture_name".to_owned(),
						some_subpattern: None,
					},
					Segment::Static(".static".to_owned()),
				],
			),
			(
				"{capture_name}.{capture_name}.static",
				vec![
					Segment::Capturing {
						name: "capture_name".to_owned(),
						some_subpattern: None,
					},
					Segment::Static(".".to_owned()),
					Segment::Capturing {
						name: "capture_name".to_owned(),
						some_subpattern: None,
					},
					Segment::Static(".static".to_owned()),
				],
			),
			(
				"{capture_name:pattern}{capture_name:pattern}",
				vec![
					Segment::Capturing {
						name: "capture_name".to_owned(),
						some_subpattern: Some("pattern".to_owned()),
					},
					Segment::Capturing {
						name: "capture_name".to_owned(),
						some_subpattern: Some("pattern".to_owned()),
					},
				],
			),
			(
				"static-{capture_name:pattern}",
				vec![
					Segment::Static("static-".to_owned()),
					Segment::Capturing {
						name: "capture_name".to_owned(),
						some_subpattern: Some("pattern".to_owned()),
					},
				],
			),
			(
				"{{static-{capture_name:pattern}}}",
				vec![
					Segment::Static("{static-".to_owned()),
					Segment::Capturing {
						name: "capture_name".to_owned(),
						some_subpattern: Some("pattern".to_owned()),
					},
					Segment::Static("}".to_owned()),
				],
			),
			(
				"{capture_name:pattern}-static",
				vec![
					Segment::Capturing {
						name: "capture_name".to_owned(),
						some_subpattern: Some("pattern".to_owned()),
					},
					Segment::Static("-static".to_owned()),
				],
			),
			(
				"{{{capture_name:pattern}-static}}",
				vec![
					Segment::Static("{".to_owned()),
					Segment::Capturing {
						name: "capture_name".to_owned(),
						some_subpattern: Some("pattern".to_owned()),
					},
					Segment::Static("-static}".to_owned()),
				],
			),
		];

		for case in cases {
			dbg!(case.0);

			let segments = super::split(case.0.chars().peekable());
			assert_eq!(segments, case.1);
		}
	}

	#[test]
	#[should_panic(expected = "incomplete pattern")]
	fn split_incomplete_pattern() {
		let pattern = "static{capture_name";
		super::split(pattern.chars().peekable());
	}

	#[test]
	#[should_panic(expected = "no closing brace")]
	fn split_no_closing_parenthesis() {
		let pattern = "static{pattern:";
		super::split(pattern.chars().peekable());
	}

	#[test]
	#[should_panic(expected = "empty regex subpattern")]
	fn split_empty_regex_subpattern() {
		let pattern = "{capture_name:}{capture_name:pattern}";
		super::split(pattern.chars().peekable());
	}

	#[test]
	#[should_panic(expected = "cannot have a named capture group")]
	fn split_regex_subpattern_with_named_capture_group1() {
		let pattern = "{capture_name:(?P<name>abc)}{capture_name:pattern}";
		super::split(pattern.chars().peekable());
	}

	#[test]
	#[should_panic(expected = "cannot have a named capture group")]
	fn split_regex_subpattern_with_named_capture_group2() {
		let pattern = "{capture_name:(?<name>abc)}{capture_name:pattern}";
		super::split(pattern.chars().peekable());
	}

	#[test]
	fn parse() {
		let cases = [
			("static", Pattern::Static("static".into())),
			(
				"{{not_capture_name:pattern}}",
				Pattern::Static(
					Cow::<str>::from(percent_encode(b"{not_capture_name:pattern}", ASCII_SET)).into(),
				),
			),
			(
				"{{not_capture_name}}",
				Pattern::Static(Cow::<str>::from(percent_encode(b"{not_capture_name}", ASCII_SET)).into()),
			),
			(
				"static:{{not_capture_name}}",
				Pattern::Static(
					Cow::<str>::from(percent_encode(b"static:{not_capture_name}", ASCII_SET)).into(),
				),
			),
			(
				"{{not_capture_name}}:static",
				Pattern::Static(
					Cow::<str>::from(percent_encode(b"{not_capture_name}:static", ASCII_SET)).into(),
				),
			),
			(
				"{capture_name:pattern}",
				Pattern::Regex(
					RegexNames::new(
						Regex::new(r"\A(?P<capture_name>pattern)\z")
							.unwrap()
							.capture_names(),
					),
					Regex::new(r"\A(?P<capture_name>pattern)\z").unwrap(),
				),
			),
			(
				"static{capture_name:pattern}.static{{not_capture_name}}",
				Pattern::Regex(
					RegexNames::new(
						Regex::new(r"\Astatic(?P<capture_name>pattern)\.static\{not_capture_name\}\z")
							.unwrap()
							.capture_names(),
					),
					Regex::new(r"\Astatic(?P<capture_name>pattern)\.static\{not_capture_name\}\z").unwrap(),
				),
			),
			(
				"static{capture_name_1}.static{capture_name_2}",
				Pattern::Regex(
					RegexNames::new(
						Regex::new(r"\Astatic(?P<capture_name_1>[^.]+)\.static(?P<capture_name_2>.+)\z")
							.unwrap()
							.capture_names(),
					),
					Regex::new(r"\Astatic(?P<capture_name_1>[^.]+)\.static(?P<capture_name_2>.+)\z").unwrap(),
				),
			),
			(
				"{{not_capture_name:pattern}}{capture_name}.{{",
				Pattern::Regex(
					RegexNames::new(
						Regex::new(r"\A\{not_capture_name:pattern\}(?P<capture_name>[^.]+)\.\{\z")
							.unwrap()
							.capture_names(),
					),
					Regex::new(r"\A\{not_capture_name:pattern\}(?P<capture_name>[^.]+)\.\{\z").unwrap(),
				),
			),
			("{capture_name}", Pattern::Wildcard("capture_name".into())),
		];

		for case in cases {
			let pattern = Pattern::parse(case.0);
			println!(
				"case: {}\n\tpattern:  {}\n\texpected: {}",
				case.0, pattern, &case.1
			);

			assert!(pattern.compare(&case.1) == Similarity::Same);
		}
	}

	#[test]
	#[should_panic(expected = "empty regex capture name")]
	fn parse_empty_regex_capture_name() {
		Pattern::parse("{:pattern}");
	}

	#[test]
	#[should_panic(expected = "no closing brace")]
	fn parse_no_closing_brace() {
		Pattern::parse("{name:");
	}

	#[test]
	#[should_panic(expected = "empty regex subpattern")]
	fn parse_empty_regex_subpattern() {
		Pattern::parse("{name:}");
	}

	#[test]
	#[should_panic(expected = "a wildcard must be an only or the last segment")]
	fn parse_wildcard_must_be_the_last_segment() {
		Pattern::parse("static{capture_name:pattern}{capture_name}_");
	}

	#[test]
	#[should_panic(expected = "or it must be followed by a '.'")]
	fn parse_wildcard_must_be_followed_by_dot() {
		Pattern::parse("{name}static");
	}

	#[test]
	#[allow(clippy::type_complexity)]
	fn is_match() {
		struct Case<'a> {
			kind: Kind,
			pattern: &'a str,
			matching: &'a [(&'a str, Option<&'a str>)],
			nonmatching: &'a [&'a str],
		}

		enum Kind {
			Static,
			Regex,
			Wildcard,
		}

		let cases = [
			Case {
				kind: Kind::Static,
				pattern: "login",
				matching: &[("login", None)],
				nonmatching: &["logout"],
			},
			Case {
				kind: Kind::Regex,
				pattern: r"{prefix:A|B|C}{number:\d{5}}",
				matching: &[
					("A12345", Some("regex params: [prefix:A, number:12345]")),
					("B54321", Some("regex params: [prefix:B, number:54321]")),
					("C11111", Some("regex params: [prefix:C, number:11111]")),
				],
				nonmatching: &["D12345", "0ABCDEF", "AA12345", "B123456", "C1234", "AB1234"],
			},
			Case {
				kind: Kind::Regex,
				pattern: "{brand:.+} ({model:.+})",
				matching: &[
					(
						"Audi (e-tron GT)",
						Some("regex params: [brand:Audi, model:e-tron GT]"),
					),
					(
						"Volvo (XC40 Recharge)",
						Some("regex params: [brand:Volvo, model:XC40 Recharge]"),
					),
				],
				nonmatching: &["Audi(Q8)", "Volvo C40", "Audi [A4]"],
			},
			Case {
				kind: Kind::Regex,
				pattern: "{brand}.{model}",
				matching: &[
					(
						"Audi.e-tron GT",
						Some("regex params: [brand:Audi, model:e-tron GT]"),
					),
					(
						"Volvo.XC40 Recharge",
						Some("regex params: [brand:Volvo, model:XC40 Recharge]"),
					),
				],
				nonmatching: &["Audi(Q8)", "Volvo C40", "Audi,A4"],
			},
			Case {
				kind: Kind::Wildcard,
				pattern: "{card}",
				matching: &[
					(
						"king of clubs",
						Some("wildcard param: [card:king of clubs]"),
					),
					(
						"queen of hearts",
						Some("wildcard param: [card:queen of hearts]"),
					),
				],
				nonmatching: &[],
			},
		];

		for case in cases {
			let pattern = Pattern::parse(case.pattern);
			// dbg!(&pattern);

			for (text, expected_params) in case.matching {
				let mut params_list = ParamsList::new();

				match case.kind {
					Kind::Static => assert!(pattern.is_static_match(text).unwrap()),
					Kind::Regex => {
						assert!(pattern.is_regex_match(*text, &mut params_list).unwrap());

						let params = params_list.iter().next().unwrap();
						assert_eq!(params.to_string(), expected_params.unwrap())
					}
					Kind::Wildcard => {
						assert!(pattern
							.is_wildcard_match(Cow::from(*text), &mut params_list)
							.unwrap());

						let params = params_list.iter().next().unwrap();
						assert_eq!(params.to_string(), expected_params.unwrap())
					}
				}
			}

			let mut params_list = ParamsList::new();
			for text in case.nonmatching {
				match case.kind {
					Kind::Static => assert!(!pattern.is_static_match(*text).unwrap()),
					Kind::Regex => {
						assert!(!pattern.is_regex_match(*text, &mut params_list).unwrap());
					}
					Kind::Wildcard => {
						assert!(!pattern
							.is_wildcard_match(Cow::from(*text), &mut params_list)
							.unwrap());
					}
				}
			}
		}
	}
}
