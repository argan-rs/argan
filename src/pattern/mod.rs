use std::{
	borrow::Cow,
	fmt::Display,
	iter::Peekable,
	slice,
	str::Chars,
	sync::{Arc, OnceLock},
};

use percent_encoding::{percent_encode, NON_ALPHANUMERIC};
use regex::{CaptureLocations, CaptureNames, Regex};

// --------------------------------------------------

mod deserializers;

pub(crate) use deserializers::FromParamsList;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// static:		static, resource
// regex:			$name:@capture_name(pattern)escaped@(pattern)escaped@capture_name(pattern)
// wildcard:	*name

#[derive(Debug, Clone)]
pub(crate) enum Pattern {
	Static(Arc<str>),
	Regex(RegexNames, Option<Regex>),
	Wildcard(Arc<str>),
}

impl Pattern {
	pub(crate) fn parse(pattern: &str) -> Pattern {
		// Wildcard pattern.
		if let Some(wildcard_name) = pattern.strip_prefix('*') {
			if wildcard_name.is_empty() {
				panic!("empty wildcard pattern name")
			}

			return Pattern::Wildcard(wildcard_name.into());
		}

		let mut chars = pattern.chars().peekable();

		// Regex pattern.
		if let Some('$') = chars.peek() {
			chars.next(); // discarding '$'

			let (name, some_delimiter) = split_at_delimiter(&mut chars, |ch| ch == ':');
			if name.is_empty() {
				panic!("empty regex pattern name")
			}

			if some_delimiter.is_none() {
				return Pattern::Regex(RegexNames::new(name, None), None);
			}

			let mut segments = split(chars);

			if segments.is_empty() {
				panic!("incomplete regex pattern")
			}

			if let [segment] = segments.as_mut_slice() {
				match segment {
					Segment::Static(_) => {
						panic!("regex pattern must have at least one capturing or non-capturing segment",)
					}
					Segment::Capturing {
						some_name,
						subpattern,
					} => {
						let capture_name = if let Some(capture_name) = some_name.take() {
							capture_name
						} else {
							name.clone()
						};

						let regex_subpattern = format!(r"\A(?P<{}>{})\z", capture_name, subpattern);
						match Regex::new(&regex_subpattern) {
							Ok(regex) => {
								let capture_names = regex.capture_names();

								return Pattern::Regex(RegexNames::new(name, Some(capture_names)), Some(regex));
							}
							Err(error) => panic!("{}", error),
						}
					}
				};
			}

			let mut regex_pattern = "\\A".to_owned();

			let mut nameless_capturing_segment_exists = false;
			let mut capturing_segments_count = 0;
			for segment in segments {
				match segment {
					Segment::Static(mut subpattern) => {
						subpattern = regex::escape(subpattern.as_ref());
						regex_pattern.push_str(&subpattern);
					}
					Segment::Capturing {
						some_name,
						mut subpattern,
					} => {
						subpattern = if let Some(capture_name) = some_name {
							if nameless_capturing_segment_exists {
								panic!(
									"regex pattern without a capture name cannot have multiple capturing segments",
								)
							}

							let subpattern = format!("(?P<{}>{})", &capture_name, subpattern);

							subpattern
						} else {
							if capturing_segments_count > 0 {
								panic!("regex pattern with multiple capturing segments cannot omit a capture name")
							}

							nameless_capturing_segment_exists = true;

							format!("(?P<{}>{})", &name, subpattern)
						};

						capturing_segments_count += 1;
						regex_pattern.push_str(&subpattern);
					}
				}
			}

			regex_pattern.push_str("\\z");
			match Regex::new(&regex_pattern) {
				Ok(regex) => {
					let capture_names = regex.capture_names();

					return Pattern::Regex(RegexNames::new(name, Some(capture_names)), Some(regex));
				}
				Err(error) => panic!("{}", error),
			}
		}

		if let Some('\\') = chars.peek() {
			let mut buf = String::new();

			while let Some(ch) = chars.next() {
				if ch == '\\' {
					if let Some('*' | '$') = chars.peek() {
						break;
					}

					buf.push(ch);
				} else {
					buf.push(ch);
					break;
				}
			}

			buf.extend(chars);
			let encoded_static_pattern =
				Cow::<str>::from(percent_encode(buf.as_bytes(), NON_ALPHANUMERIC));

			return Pattern::Static(encoded_static_pattern.into());
		}

		let encoded_static_pattern =
			Cow::<str>::from(percent_encode(pattern.as_bytes(), NON_ALPHANUMERIC));

		Pattern::Static(encoded_static_pattern.into())
	}

	#[inline(always)]
	pub(crate) fn name(&self) -> Option<&str> {
		match self {
			Pattern::Static(_) => None,
			Pattern::Regex(names, _) => Some(names.pattern_name()),
			Pattern::Wildcard(name) => Some(name.as_ref()),
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
			} else {
				return Some(false);
			}
		}

		None
	}

	#[inline]
	pub(crate) fn is_regex_match(&self, text: &str, params_list: &mut ParamsList) -> Option<bool> {
		if let Self::Regex(names, Some(regex)) = self {
			let mut capture_locations = regex.capture_locations();
			if regex.captures_read(&mut capture_locations, text).is_some() {
				params_list.push(Params::with_regex_captures(
					names.clone(),
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

			return Some(true);
		} else {
			return None;
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
			Pattern::Regex(names, some_regex) => {
				if let Pattern::Regex(other_names, some_other_regex) = other {
					if (some_regex.is_none() && some_other_regex.is_none())
						|| some_regex.as_ref().is_some_and(|regex| {
							if let Some(other_regex) = some_other_regex.as_ref() {
								regex.as_str() == other_regex.as_str()
							} else {
								false
							}
						}) {
						if names.pattern_name() == other_names.pattern_name() {
							return Similarity::Same;
						} else {
							return Similarity::DifferentNames;
						}
					}
				}
			}
			Pattern::Wildcard(name) => {
				if let Pattern::Wildcard(other_name) = other {
					if name == other_name {
						return Similarity::Same;
					} else {
						return Similarity::DifferentNames;
					}
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
			Pattern::Static(pattern) => write!(f, "{}", pattern),
			Pattern::Regex(names, Some(regex)) => write!(f, "${}:@({})", names.pattern_name(), regex),
			Pattern::Regex(names, None) => write!(f, "${}", names.pattern_name()),
			Pattern::Wildcard(name) => write!(f, "*{}", name),
		}
	}
}

// -------------------------

impl Default for Pattern {
	fn default() -> Self {
		EMPTY_STATIC
			.get_or_init(|| Pattern::Static("".into()))
			.clone()
	}
}

static EMPTY_STATIC: OnceLock<Pattern> = OnceLock::new();

// --------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RegexNames(Arc<[(Box<str>, usize)]>);

impl RegexNames {
	fn new(pattern_name: String, some_capture_names: Option<CaptureNames>) -> Self {
		// The first element is the name of the entire regex pattern.
		let mut names = vec![(Box::from(pattern_name), 0)];

		if let Some(capture_names) = some_capture_names {
			for (i, some_capture_name) in capture_names.enumerate() {
				if let Some(capture_name) = some_capture_name {
					names.push((Box::from(capture_name), i));
				}
			}

			return Self(Arc::from(names));
		}

		// When regex pattern is not given we only need a pattern name.
		Self(Arc::from(names))
	}

	#[inline]
	pub(crate) fn pattern_name(&self) -> &str {
		&self.0[0].0
	}

	#[inline]
	fn get(&self, index: usize) -> Option<(&str, usize)> {
		self.0.get(index).map(|elem| (elem.0.as_ref(), elem.1))
	}

	#[inline]
	fn len(&self) -> usize {
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
				string.push_str(&format!("pattern ${}: [", regex_names.pattern_name()));

				let mut first = true;
				for (name, index) in regex_names.as_ref().iter().skip(1) {
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
				string.push_str(&format!("pattern *{}: [{}]", name, value));
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
		some_name: Option<String>,
		subpattern: String,
	},
}

#[inline]
fn split(mut chars: Peekable<Chars>) -> Vec<Segment> {
	let mut slices = Vec::new();
	let mut parsing_static = true;

	loop {
		if parsing_static {
			let (static_segment, some_delimiter) = split_at_delimiter(&mut chars, |ch| ch == '@');
			if !static_segment.is_empty() {
				slices.push(Segment::Static(static_segment));
			}

			if some_delimiter.is_some() {
				parsing_static = false
			} else {
				break;
			}
		} else {
			let (name, some_delimiter) = split_at_delimiter(&mut chars, |ch| ch == '(');

			let Some(_) = some_delimiter else {
				panic!("incomplete pattern")
			};

			let some_name = if name.is_empty() { None } else { Some(name) };

			let Some(subpattern) = split_off_subpattern(&mut chars) else {
				panic!("no closing parenthesis of regex subpattern found")
			};

			if subpattern.is_empty() {
				panic!("empty regex subpattern")
			}

			slices.push(Segment::Capturing {
				some_name,
				subpattern,
			});
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
) -> (String, Option<char>) {
	let mut buf = String::new();
	let mut unescaped = true;

	while let Some(ch) = chars.next() {
		if delimiter(ch) {
			if unescaped {
				return (buf, Some(ch));
			}

			unescaped = true;
		} else if ch == '\\' {
			if let Some(next_ch) = chars.peek() {
				if delimiter(*next_ch) {
					unescaped = false;

					continue;
				}
			}
		}

		buf.push(ch);
	}

	(buf, None)
}

// Returns the regex subpattern if the end of the regex segment is found. Otherwise None.
// The regex pattern may be empty if the end of the regex segment is met right away.
fn split_off_subpattern(chars: &mut Peekable<Chars<'_>>) -> Option<String> {
	let mut subpattern = String::new();
	let mut depth = 1; // We are already inside the opened '(' bracket.
	let mut unescaped = true;
	let mut in_character_class = -1i8;
	let mut in_named_capture_group = -1i8;

	while let Some(ch) = chars.next() {
		if ch == ')' && unescaped && in_character_class < 0 {
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
			'(' => {
				if unescaped || in_character_class < 0 {
					depth += 1;

					if let Some('?') = chars.peek() {
						in_named_capture_group += 1;
					}
				}
			}
			'\\' => {
				if unescaped {
					if let Some('\\' | '[' | ']' | '(' | ')') = chars.peek() {
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
	DifferentNames,
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
		let mut pattern = "escaped@capture(regex)".chars().peekable();

		let (escaped, some_delimiter) =
			super::split_at_delimiter(&mut pattern, |ch| ch == '@' || ch == '(' || ch == ')');

		assert_eq!(escaped, "escaped");
		assert_eq!(some_delimiter, Some('@'));

		let (capture, some_delimiter) =
			super::split_at_delimiter(&mut pattern, |ch| ch == '@' || ch == '(' || ch == ')');

		assert_eq!(capture, "capture");
		assert_eq!(some_delimiter, Some('('));

		let (regex, some_delimiter) =
			super::split_at_delimiter(&mut pattern, |ch| ch == '@' || ch == '(' || ch == ')');

		assert_eq!(regex, "regex");
		assert_eq!(some_delimiter, Some(')'));
	}

	#[test]
	fn split_off_subpattern() {
		let subpattern1 = r"(\d{4})-(\d{2})-(\d{2})";
		let subpattern2 = r"(.+)$";
		let subpattern3 = r"[^0-9)]+";
		let subpattern4 = r"[^])]";
		let subpattern5 = r"[^a])]";
		let pattern = format!(
			"({}):({}):({}):({}):({})",
			subpattern1, subpattern2, subpattern3, subpattern4, subpattern5,
		);

		let mut pattern = pattern.chars().peekable();
		pattern.next(); // We must remove the opening parenthesis.

		let subpattern = super::split_off_subpattern(&mut pattern);
		assert_eq!(subpattern, Some(subpattern1.to_owned()));
		println!("subpattern 1: {}", subpattern.unwrap());

		assert_eq!(pattern.next(), Some(':'));
		assert_eq!(pattern.next(), Some('('));

		let subpattern = super::split_off_subpattern(&mut pattern);
		assert_eq!(subpattern, Some(subpattern2.to_owned()));
		println!("subpattern 2: {}", subpattern.unwrap());

		assert_eq!(pattern.next(), Some(':'));
		assert_eq!(pattern.next(), Some('('));

		let subpattern = super::split_off_subpattern(&mut pattern);
		assert_eq!(subpattern, Some(subpattern3.to_owned()));
		println!("subpattern 3: {}", subpattern.unwrap());

		assert_eq!(pattern.next(), Some(':'));
		assert_eq!(pattern.next(), Some('('));

		let subpattern = super::split_off_subpattern(&mut pattern);
		assert_eq!(subpattern, Some(subpattern4.to_owned()));
		println!("subpattern 4: {}", subpattern.unwrap());

		assert_eq!(pattern.next(), Some(':'));
		assert_eq!(pattern.next(), Some('('));

		let subpattern = super::split_off_subpattern(&mut pattern);
		assert_ne!(subpattern, Some(subpattern5.to_owned()));
		println!("subpattern 5: {}", subpattern.unwrap());

		assert_eq!(pattern.next(), Some(']'));
	}

	#[test]
	fn split() {
		let cases = [
			(
				"static@capture_name(pattern)-@(pattern)",
				vec![
					Segment::Static("static".to_owned()),
					Segment::Capturing {
						some_name: Some("capture_name".to_owned()),
						subpattern: "pattern".to_owned(),
					},
					Segment::Static("-".to_owned()),
					Segment::Capturing {
						some_name: None,
						subpattern: "pattern".to_owned(),
					},
				],
			),
			(
				"static@(pattern)static",
				vec![
					Segment::Static("static".to_owned()),
					Segment::Capturing {
						some_name: None,
						subpattern: "pattern".to_owned(),
					},
					Segment::Static("static".to_owned()),
				],
			),
			(
				"@capture_name(pattern)@capture_name(pattern)",
				vec![
					Segment::Capturing {
						some_name: Some("capture_name".to_owned()),
						subpattern: "pattern".to_owned(),
					},
					Segment::Capturing {
						some_name: Some("capture_name".to_owned()),
						subpattern: "pattern".to_owned(),
					},
				],
			),
			(
				"@capture_name(pattern)-static",
				vec![
					Segment::Capturing {
						some_name: Some("capture_name".to_owned()),
						subpattern: "pattern".to_owned(),
					},
					Segment::Static("-static".to_owned()),
				],
			),
		];

		for case in cases {
			let segments = super::split(case.0.chars().peekable());
			assert_eq!(segments, case.1);
		}
	}

	#[test]
	#[should_panic(expected = "incomplete pattern")]
	fn split_incomplete_pattern() {
		let pattern = "static@capture_name";
		println!("pattern: {}", pattern);
		super::split(pattern.chars().peekable());
	}

	#[test]
	#[should_panic(expected = "no closing parenthesis")]
	fn split_no_closing_parenthesis() {
		let pattern = "static@(pattern";
		println!("pattern: {}", pattern);
		super::split(pattern.chars().peekable());
	}

	#[test]
	#[should_panic(expected = "empty regex subpattern")]
	fn split_empty_regex_subpattern() {
		let pattern = "@capture_name()@capture_name(pattern)";
		println!("pattern: {}", pattern);
		super::split(pattern.chars().peekable());
	}

	#[test]
	#[should_panic(expected = "cannot have a named capture group")]
	fn split_regex_subpattern_with_named_capture_group1() {
		let pattern = "@capture_name((?P<name>abc))@capture_name(pattern)";
		println!("pattern: {}", pattern);
		super::split(pattern.chars().peekable());
	}

	#[test]
	#[should_panic(expected = "cannot have a named capture group")]
	fn split_regex_subpattern_with_named_capture_group2() {
		let pattern = "@capture_name((?<name>abc))@capture_name(pattern)";
		println!("pattern: {}", pattern);
		super::split(pattern.chars().peekable());
	}

	#[test]
	fn parse() {
		let cases = [
			("", Pattern::Static("".into())),
			("static", Pattern::Static("static".into())),
			(
				"@capture_name(pattern)",
				Pattern::Static("@capture_name(pattern)".into()),
			),
			(
				r"\$name:@(pattern)",
				Pattern::Static("$name:@(pattern)".into()),
			),
			(
				r"\\$name:@(pattern)",
				Pattern::Static(r"\$name:@(pattern)".into()),
			),
			(r"\*wildcard", Pattern::Static("*wildcard".into())),
			(r"\\*wildcard", Pattern::Static(r"\*wildcard".into())),
			(
				"$name:@capture_name(pattern)",
				Pattern::Regex(
					RegexNames::new(
						"name".to_owned(),
						Some(
							Regex::new(r"\A(?P<capture_name>pattern)\z")
								.unwrap()
								.capture_names(),
						),
					),
					Some(Regex::new(r"\A(?P<capture_name>pattern)\z").unwrap()),
				),
			),
			(
				"$name:@(pattern)",
				Pattern::Regex(
					RegexNames::new(
						"name".to_owned(),
						Some(
							Regex::new(r"\A(?P<name>pattern)\z")
								.unwrap()
								.capture_names(),
						),
					),
					Some(Regex::new(r"\A(?P<name>pattern)\z").unwrap()),
				),
			),
			(
				"$name",
				Pattern::Regex(RegexNames::new("name".to_owned(), None), None),
			),
			(
				"$@capture_name(pattern)",
				Pattern::Regex(
					RegexNames::new("@capture_name(pattern)".to_owned(), None),
					None,
				),
			),
			(
				"$name:static@capture_name(pattern).static[pattern]",
				Pattern::Regex(
					RegexNames::new(
						"name".to_owned(),
						Some(
							Regex::new(r"\Astatic(?P<capture_name>pattern)\.static\[pattern\]\z")
								.unwrap()
								.capture_names(),
						),
					),
					Some(Regex::new(r"\Astatic(?P<capture_name>pattern)\.static\[pattern\]\z").unwrap()),
				),
			),
			(
				r"$name:\@capture_name(pattern)@(pattern)\@",
				Pattern::Regex(
					RegexNames::new(
						"name".to_owned(),
						Some(
							Regex::new(r"\A@capture_name\(pattern\)(?P<name>pattern)@\z")
								.unwrap()
								.capture_names(),
						),
					),
					Some(Regex::new(r"\A@capture_name\(pattern\)(?P<name>pattern)@\z").unwrap()),
				),
			),
			("*wildcard", Pattern::Wildcard(Arc::from("wildcard"))),
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
	#[should_panic(expected = "empty regex pattern name")]
	fn parse_empty_regex_pattern_name() {
		Pattern::parse("$:@capture_name(pattern)");
	}

	#[test]
	#[should_panic(expected = "incomplete regex pattern")]
	fn parse_incomplete_regex_pattern() {
		Pattern::parse("$name:");
	}

	#[test]
	#[should_panic(expected = "must have at least one capturing or non-capturing segment")]
	fn parse_regex_subpattern() {
		Pattern::parse("$name:static");
	}

	#[test]
	#[should_panic(expected = "multiple capturing segments cannot omit a capture name")]
	fn parse_without_capture_name1() {
		Pattern::parse("$name:static@capture_name(pattern)@(pattern)");
	}

	#[test]
	#[should_panic(expected = "without a capture name cannot have multiple capturing segments")]
	fn parse_without_capture_name2() {
		Pattern::parse("$name:static@(pattern)@capture_name(pattern)");
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
				pattern: r"$id:@prefix(A|B|C)@number(\d{5})",
				matching: &[
					("A12345", Some("pattern $id: [prefix:A, number:12345]")),
					("B54321", Some("pattern $id: [prefix:B, number:54321]")),
					("C11111", Some("pattern $id: [prefix:C, number:11111]")),
				],
				nonmatching: &["D12345", "0ABCDEF", "AA12345", "B123456", "C1234", "AB1234"],
			},
			Case {
				kind: Kind::Regex,
				pattern: r"$car:@brand(.+) (@model(.+))",
				matching: &[
					(
						"Audi (e-tron GT)",
						Some("pattern $car: [brand:Audi, model:e-tron GT]"),
					),
					(
						"Volvo (XC40 Recharge)",
						Some("pattern $car: [brand:Volvo, model:XC40 Recharge]"),
					),
				],
				nonmatching: &["Audi(Q8)", "Volvo C40", "Audi [A4]"],
			},
			Case {
				kind: Kind::Wildcard,
				pattern: "*card",
				matching: &[
					("king of clubs", Some("pattern *card: [king of clubs]")),
					("queen of hearts", Some("pattern *card: [queen of hearts]")),
				],
				nonmatching: &[],
			},
		];

		for case in cases {
			let pattern = Pattern::parse(case.pattern);
			println!("pattern: {}", pattern);

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

	// #[test]
	// #[allow(clippy::type_complexity)]
	// fn params_next() {
	// 	struct Case<'a> {
	// 		pattern: &'a str,
	// 		matching: &'a [(&'a str, Option<&'a [(&'a str, &'a str)]>)],
	// 	}
	//
	// 	let cases = [
	// 		Case { pattern: "login", matching: &[("login", None)] },
	// 		Case {
	// 			pattern: r"$id:@prefix(A|B|C)@number(\d{5})@suffix([A-Z]?)",
	// 			matching: &[
	// 				("A12345Z", Some(&[("prefix", "A"), ("number", "12345"), ("suffix", "Z")])),
	// 				("B54321", Some(&[("prefix", "B"), ("number", "54321"), ("suffix", "")])),
	// 				("C11111", Some(&[("prefix", "C"), ("number", "11111"), ("suffix", "")])),
	// 			],
	// 		},
	// 		Case {
	// 			pattern: r"$car:@brand(.+) (@model(.+))",
	// 			matching: &[
	// 				("Audi (e-tron GT)", Some(&[("brand", "Audi"), ("model", "e-tron GT")])),
	// 				("Volvo (XC40 Recharge)", Some(&[("brand", "Volvo"), ("model", "XC40 Recharge")])),
	// 			],
	// 		},
	// 		Case {
	// 			pattern: "*card",
	// 			matching: &[
	// 				("king of clubs", Some(&[("card", "king of clubs")])),
	// 				("queen of hearts", Some(&[("card", "queen of hearts")])),
	// 			],
	// 		},
	// 	];
	//
	// 	for case in cases {
	// 		let pattern = Pattern::parse(case.pattern);
	// 		println!("pattern: {}", pattern);
	//
	// 		for (text, some_expected_outcome) in case.matching {
	// 			let outcome = pattern.is_match(text);
	//
	// 			if let Some(expected_outcome) = some_expected_outcome {
	// 				let mut params = match outcome {
	// 					MatchOutcome::Dynamic(params) => params,
	// 					_ => panic!()
	// 				};
	//
	// 				for (expected_name, expected_value) in *expected_outcome {
	// 					let param = params.next().unwrap();
	// 					assert_eq!(param.name(), *expected_name);
	//
	// 					assert_eq!(param.value().or(Some("")), Some(*expected_value))
	// 				}
	// 			} else if let MatchOutcome::Static = outcome {} else {
	// 				panic!("outcome is not a static match")
	// 			}
	// 		}
	// 	}
	// }
}
