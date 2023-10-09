use std::{fmt::Display, iter::Peekable, str::Chars, sync::Arc};

use regex::{CaptureNames, Captures, Regex};

// --------------------------------------------------

mod deserializers;

pub(crate) use deserializers::FromPath;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// static:		static, resource
// regex:			$name:@capture_name(pattern)escaped@(pattern)escaped@capture_name(pattern)
// wildcard:	*name

#[derive(Clone, Debug)]
pub(crate) enum Pattern {
	Static(Arc<str>),
	Regex(Arc<str>, Option<Regex>),
	Wildcard(Arc<str>),
}

impl Pattern {
	pub fn parse(pattern: &str) -> Pattern {
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
				return Pattern::Regex(name.into(), None);
			}

			let mut segments = split(chars);

			if segments.is_empty() {
				panic!("incomplete regex pattern")
			}

			if let [segment] = segments.as_slice() {
				match segment {
					Segment::Static(_) => {
						panic!("regex pattern must have at least one capturing or non-capturing segment")
					}
					Segment::Capturing {
						some_name,
						subpattern,
					} => {
						let regex_subpattern = if let Some(capture_name) = some_name {
							format!(r"\A(?P<{}>{})\z", capture_name, subpattern)
						} else {
							format!(r"\A(?P<{}>{})\z", &name, subpattern)
						};

						let regex = Regex::new(&regex_subpattern).unwrap();

						return Pattern::Regex(name.into(), Some(regex));
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
									"regex pattern without a capture name cannot have multiple capturing segments"
								)
							}

							format!("(?P<{}>{})", capture_name, subpattern)
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
			let regex = Regex::new(&regex_pattern).unwrap();

			return Pattern::Regex(name.into(), Some(regex));
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
			return Pattern::Static(buf.into());
		}

		Pattern::Static(pattern.into())
	}

	#[inline]
	pub fn name(&self) -> Option<&str> {
		match self {
			Pattern::Static(_) => None,
			Pattern::Regex(name, _) | Pattern::Wildcard(name) => Some(name.as_ref()),
		}
	}

	#[inline]
	pub fn is_static(&self) -> bool {
		if let Pattern::Static(_) = self {
			return true;
		}

		false
	}

	#[inline]
	pub fn is_regex(&self) -> bool {
		if let Pattern::Regex(_, _) = self {
			return true;
		}

		false
	}

	#[inline]
	pub fn is_wildcard(&self) -> bool {
		if let Pattern::Wildcard(_) = self {
			return true;
		}

		false
	}

	pub fn is_match<'p, 's: 'p>(&'s self, text: &'p str) -> MatchOutcome<'p> {
		if !text.is_empty() {
			match self {
				Pattern::Static(pattern) => {
					if pattern.as_ref() == text {
						return MatchOutcome::Static;
					}
				}
				Pattern::Regex(name, some_regex) => {
					if let Some(regex) = some_regex {
						if let Some(captures) = regex.captures(text) {
							let mut capture_names = regex.capture_names();

							return MatchOutcome::Dynamic(Params::Regex(
								name.as_ref(),
								capture_names.peekable(),
								captures,
							));
						}
					} else {
						panic!("regex pattern has only a name")
					}
				}
				Pattern::Wildcard(name) => {
					return MatchOutcome::Dynamic(Params::Wildcard(name.as_ref(), Some(text)))
				}
			}
		}

		MatchOutcome::None
	}

	pub fn compare(&self, other: &Self) -> Similarity {
		match self {
			Pattern::Static(pattern) => {
				if let Pattern::Static(other_pattern) = other {
					if pattern == other_pattern {
						return Similarity::Same;
					}
				}
			}
			Pattern::Regex(name, some_regex) => {
				if let Pattern::Regex(other_name, some_other_regex) = other {
					if (some_regex.is_none() && some_other_regex.is_none())
						|| some_regex.as_ref().is_some_and(|regex| {
							if let Some(other_regex) = some_other_regex.as_ref() {
								regex.as_str() == other_regex.as_str()
							} else {
								false
							}
						}) {
						if name == other_name {
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
			Pattern::Regex(name, Some(regex)) => write!(f, "${}:@({})", name, regex),
			Pattern::Regex(name, None) => write!(f, "${}", name),
			Pattern::Wildcard(name) => write!(f, "*{}", name),
		}
	}
}

// --------------------------------------------------

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

			let Some(delimiter) = some_delimiter else {
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

// Returns the segment before the delimiter and the delimiter. If the delimiter is
// not found then the segment contains all the chars and the returned delimiter will
// be None. If there are no more chars or the delimiter is found right away then the
// returned segment will be empty.
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

// --------------------------------------------------

#[derive(Debug)]
pub(crate) enum MatchOutcome<'p> {
	None,
	Static,
	Dynamic(Params<'p>),
}

// -------------------------

#[derive(Debug)]
pub(crate) enum Params<'p> {
	Regex(&'p str, Peekable<CaptureNames<'p>>, Captures<'p>),
	Wildcard(&'p str, Option<&'p str>),
}

impl<'p> Params<'p> {
	pub(crate) fn name(&self) -> &'p str {
		match self {
			Self::Regex(name, _, _) => name,
			Self::Wildcard(name, _) => name,
		}
	}

	// fn value(&self, name: &str) -> Option<&'p str> {
	// 	match self {
	// 		Self::Regex(_, _, captures) => captures.name(name).map(|match_value| match_value.as_str()),
	// 		Self::Wildcard(wildcard_name, value) => if name == *wildcard_name { *value } else { None },
	// 	}
	// }

	pub(crate) fn current(&mut self) -> Option<Param<'p>> {
		match self {
			Self::Regex(_, ref mut capture_names, captures) => {
				while let Some(some_name) = capture_names.peek() {
					let Some(name) = some_name else {
						capture_names.next(); // Advancing the iterator.

						continue;
					};

					let some_value = captures.name(name);

					return Some(Param::new(name, some_value.map(|value| value.as_str())));
				}

				None
			}
			Self::Wildcard(name, some_value) => {
				if some_value.is_some() {
					return Some(Param::new(name, *some_value));
				}

				None
			}
		}
	}
}

impl<'p> Iterator for Params<'p> {
	type Item = Param<'p>;

	fn next(&mut self) -> Option<Self::Item> {
		match self {
			Self::Regex(_, ref mut capture_names, captures) => {
				for some_name in capture_names.by_ref() {
					let Some(name) = some_name else { continue };

					let some_value = captures.name(name);

					return Some(Param::new(name, some_value.map(|value| value.as_str())));
				}

				None
			}
			Self::Wildcard(name, some_value) => {
				if some_value.is_some() {
					return Some(Param::new(name, some_value.take()));
				}

				None
			}
		}
	}
}

// -------------------------

#[derive(Debug)]
pub(crate) struct Param<'p> {
	name: &'p str,
	some_value: Option<&'p str>,
}

// impl Display for Param<'_> {
// 	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
// 		write!(f, "{{name: {}, some_value: {:?}}}", self.name, self.some_value)
// 	}
// }

impl<'p> Param<'p> {
	#[inline]
	pub(crate) fn new(name: &'p str, some_value: Option<&'p str>) -> Self {
		Self { name, some_value }
	}

	#[inline]
	pub(crate) fn name(&self) -> &'p str {
		self.name
	}

	#[inline]
	pub(crate) fn value(&self) -> Option<&'p str> {
		self.some_value
	}
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
			subpattern1, subpattern2, subpattern3, subpattern4, subpattern5
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
		let some_regex = Option::<&Regex>::None;
		let some_ref_regex = some_regex.as_ref();
		let cases = [
			("", Pattern::Static(Arc::from(""))),
			("static", Pattern::Static(Arc::from("static"))),
			(
				"@capture_name(pattern)",
				Pattern::Static(Arc::from("@capture_name(pattern)")),
			),
			(
				r"\$name:@(pattern)",
				Pattern::Static(Arc::from("$name:@(pattern)")),
			),
			(
				r"\\$name:@(pattern)",
				Pattern::Static(Arc::from(r"\$name:@(pattern)")),
			),
			(r"\*wildcard", Pattern::Static(Arc::from("*wildcard"))),
			(r"\\*wildcard", Pattern::Static(Arc::from(r"\*wildcard"))),
			(
				"$name:@capture_name(pattern)",
				Pattern::Regex(
					Arc::from("name"),
					Some(Regex::new(r"\A(?P<capture_name>pattern)\z").unwrap()),
				),
			),
			(
				"$name:@(pattern)",
				Pattern::Regex(
					Arc::from("name"),
					Some(Regex::new(r"\A(?P<name>pattern)\z").unwrap()),
				),
			),
			("$name", Pattern::Regex(Arc::from("name"), None)),
			(
				"$@capture_name(pattern)",
				Pattern::Regex(Arc::from("@capture_name(pattern)"), None),
			),
			(
				"$name:static@capture_name(pattern).static[pattern]",
				Pattern::Regex(
					Arc::from("name"),
					Some(Regex::new(r"\Astatic(?P<capture_name>pattern)\.static\[pattern\]\z").unwrap()),
				),
			),
			(
				r"$name:\@capture_name(pattern)@(pattern)\@",
				Pattern::Regex(
					Arc::from("name"),
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
			pattern: &'a str,
			nonmatching: &'a [&'a str],
			matching: &'a [(&'a str, Option<&'a [(&'a str, &'a str)]>)],
		}

		let cases = [
			Case {
				pattern: "login",
				matching: &[("login", None)],
				nonmatching: &["logout"],
			},
			Case {
				pattern: r"$id:@prefix(A|B|C)@number(\d{5})",
				matching: &[
					("A12345", Some(&[("prefix", "A"), ("number", "12345")])),
					("B54321", Some(&[("prefix", "B"), ("number", "54321")])),
					("C11111", Some(&[("prefix", "C"), ("number", "11111")])),
				],
				nonmatching: &["D12345", "0ABCDEF", "AA12345", "B123456", "C1234", "AB1234"],
			},
			Case {
				pattern: r"$car:@brand(.+) (@model(.+))",
				matching: &[
					(
						"Audi (e-tron GT)",
						Some(&[("brand", "Audi"), ("model", "e-tron GT")]),
					),
					(
						"Volvo (XC40 Recharge)",
						Some(&[("brand", "Volvo"), ("model", "XC40 Recharge")]),
					),
				],
				nonmatching: &["Audi(Q8)", "Volvo C40", "Audi [A4]"],
			},
			Case {
				pattern: "*card",
				matching: &[
					("king of clubs", Some(&[("card", "king of clubs")])),
					("queen of hearts", Some(&[("card", "queen of hearts")])),
				],
				nonmatching: &[],
			},
		];

		for case in cases {
			let pattern = Pattern::parse(case.pattern);
			println!("pattern: {}", pattern);

			for (text, expected_outcome) in case.matching {
				let outcome = pattern.is_match(text);

				match outcome {
					MatchOutcome::Static => assert!(expected_outcome.is_none()),
					MatchOutcome::Dynamic(Params::Regex(_, capture_names, captures)) => {
						let expected_captures = expected_outcome.unwrap();

						for (expected_capture_name, expected_capture_value) in expected_captures {
							let match_value = captures.name(expected_capture_name).unwrap();
							assert_eq!(match_value.as_str(), *expected_capture_value);
						}
					}
					MatchOutcome::Dynamic(Params::Wildcard(name, capture_value)) => {
						let expected_captures = expected_outcome.unwrap();

						for (expected_capture_name, expected_capture_value) in expected_captures {
							assert_eq!(capture_value.unwrap(), *expected_capture_value);
						}
					}
					_ => panic!("Outcome::None"),
				}
			}

			for text in case.nonmatching {
				let outcome = pattern.is_match(text);
				match outcome {
					MatchOutcome::None => continue,
					_ => panic!("{:?}", outcome),
				}
			}
		}
	}

	#[test]
	#[allow(clippy::type_complexity)]
	fn params_next() {
		struct Case<'a> {
			pattern: &'a str,
			matching: &'a [(&'a str, Option<&'a [(&'a str, &'a str)]>)],
		}

		let cases = [
			Case {
				pattern: "login",
				matching: &[("login", None)],
			},
			Case {
				pattern: r"$id:@prefix(A|B|C)@number(\d{5})@suffix([A-Z]?)",
				matching: &[
					(
						"A12345Z",
						Some(&[("prefix", "A"), ("number", "12345"), ("suffix", "Z")]),
					),
					(
						"B54321",
						Some(&[("prefix", "B"), ("number", "54321"), ("suffix", "")]),
					),
					(
						"C11111",
						Some(&[("prefix", "C"), ("number", "11111"), ("suffix", "")]),
					),
				],
			},
			Case {
				pattern: r"$car:@brand(.+) (@model(.+))",
				matching: &[
					(
						"Audi (e-tron GT)",
						Some(&[("brand", "Audi"), ("model", "e-tron GT")]),
					),
					(
						"Volvo (XC40 Recharge)",
						Some(&[("brand", "Volvo"), ("model", "XC40 Recharge")]),
					),
				],
			},
			Case {
				pattern: "*card",
				matching: &[
					("king of clubs", Some(&[("card", "king of clubs")])),
					("queen of hearts", Some(&[("card", "queen of hearts")])),
				],
			},
		];

		for case in cases {
			let pattern = Pattern::parse(case.pattern);
			println!("pattern: {}", pattern);

			for (text, some_expected_outcome) in case.matching {
				let outcome = pattern.is_match(text);

				if let Some(expected_outcome) = some_expected_outcome {
					let mut params = match outcome {
						MatchOutcome::Dynamic(params) => params,
						_ => panic!(),
					};

					for (expected_name, expected_value) in *expected_outcome {
						let param = params.next().unwrap();
						assert_eq!(param.name(), *expected_name);

						assert_eq!(param.value().or(Some("")), Some(*expected_value))
					}
				} else if let MatchOutcome::Static = outcome {
				} else {
					panic!("outcome is not a static match")
				}
			}
		}
	}
}
