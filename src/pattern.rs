use std::{fmt::Display, iter::Peekable, str::Chars, sync::Arc};

use regex::Regex;

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
	pub fn parse(pattern: &str) -> Result<Pattern, &'static str /*TODO*/> {
		// Wildcard pattern.
		if let Some(wildcard_name) = pattern.strip_prefix('*') {
			if wildcard_name.is_empty() {
				return Err("empty wildcard pattern name");
			}

			return Ok(Pattern::Wildcard(wildcard_name.into()));
		}

		let mut chars = pattern.chars().peekable();

		// Regex pattern.
		if let Some('$') = chars.peek() {
			chars.next(); // discarding '$'

			let (name, some_delimiter) = split_at_delimiter(&mut chars, |ch| ch == ':');
			if name.is_empty() {
				return Err("empty regex pattern name");
			}

			if some_delimiter.is_none() {
				return Ok(Pattern::Regex(name.into(), None));
			}

			let mut segments = split(chars)?; // TODO: Return an error.

			if segments.is_empty() {
				return Err("incomplete regex pattern");
			}

			if let [segment] = segments.as_slice() {
				match segment {
					Segment::Static(pattern) => {
						return Err("regex pattern must have at least one capturing segment")
					}
					Segment::Capturing {
						some_name,
						subpattern,
					} => {
						let regex_pattern = if let Some(capture_name) = some_name {
							format!(r"\A(?P<{}>{})\z", capture_name, subpattern)
						} else {
							format!(r"\A({})\z", subpattern)
						};

						let regex = Regex::new(&regex_pattern).unwrap();

						return Ok(Pattern::Regex(name.into(), Some(regex)));
					}
				};
			}

			let mut regex_pattern = "\\A".to_owned();

			let mut nameless_capturing_segment_exists = false;
			let mut capturing_segments_count = 0;
			for segment in segments {
				match segment {
					Segment::Static(mut pattern) => {
						pattern = regex::escape(pattern.as_ref());
						regex_pattern.push_str(&pattern);
					}
					Segment::Capturing {
						some_name,
						subpattern,
					} => {
						let regex_subpattern = if let Some(capture_name) = some_name {
							if nameless_capturing_segment_exists {
								return Err(
									"regex pattern without a capture name cannot have multiple capturing segments",
								);
							}

							format!("(?P<{}>{})", capture_name, subpattern)
						} else {
							if capturing_segments_count > 0 {
								return Err(
									"regex pattern with multiple capturing segments cannot omit a capture name",
								);
							}

							nameless_capturing_segment_exists = true;
							format!("({})", subpattern)
						};

						capturing_segments_count += 1;
						regex_pattern.push_str(&regex_subpattern);
					}
				}
			}

			regex_pattern.push_str("\\z");
			let regex = Regex::new(&regex_pattern).unwrap();

			return Ok(Pattern::Regex(name.into(), Some(regex)));
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
			return Ok(Pattern::Static(buf.into()));
		}

		Ok(Pattern::Static(pattern.into()))
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

	// TODO: ???
	pub fn is_match(&self, text: &str) -> bool {
		match self {
			Pattern::Static(pattern) => pattern.as_ref() == text,
			Pattern::Regex(_, some_regex) => {
				if let Some(regex) = some_regex {
					regex.is_match(text)
				} else {
					panic!("regex pattern has no regex")
				}
			}
			_ => true,
		}
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
fn split(mut chars: Peekable<Chars>) -> Result<Vec<Segment>, &'static str /*TODO*/> {
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
				return Err("incomplete pattern");
			};

			let some_name = if name.is_empty() { None } else { Some(name) };

			let Some(subpattern) = split_off_subpattern(&mut chars) else {
				return Err("incomplete pattern");
			};

			if subpattern.is_empty() {
				return Err("empty regex subpattern");
			}

			slices.push(Segment::Capturing {
				some_name,
				subpattern,
			});
			parsing_static = true;
		}
	}

	Ok(slices)
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
					// TODO: Must be tested!
					if let Some(']') = chars.peek() {
						unescaped = false;

						continue;
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
#[derive(PartialEq)]
pub(crate) enum Similarity {
	Different,
	DifferentNames,
	Same,
}

// --------------------------------------------------------------------------------

pub(crate) fn patterns_to_string(patterns: &Vec<Pattern>) -> String {
	let mut string = String::new();
	for pattern in patterns {
		string.push('/');
		string.push_str(&pattern.to_string());
	}

	string
}

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

#[cfg(test)]
mod test {
	use super::*;

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
		let subpattern1 = r"(?<year>\d{4})-(?<month>\d{2})-(?<day>\d{2})";
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
			let segments = super::split(case.0.chars().peekable()).unwrap();
			assert_eq!(segments, case.1);
		}

		let cases = [
			"static@capture_name",
			"static@(pattern",
			"@capture_name()@capture_name(pattern)",
		];

		for case in cases {
			println!("case: {}", case);
			let result = super::split(case.chars().peekable());
			assert!(result.is_err());
		}
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
					Some(Regex::new(r"\A(pattern)\z").unwrap()),
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
					Some(Regex::new(r"\A@capture_name\(pattern\)(pattern)@\z").unwrap()),
				),
			),
			("*wildcard", Pattern::Wildcard(Arc::from("wildcard"))),
		];

		for case in cases {
			let result = Pattern::parse(case.0);
			println!(
				"case: {}\n\tpattern:  {}\n\texpected: {}",
				case.0,
				result.as_ref().unwrap(),
				&case.1
			);

			assert!(result.unwrap().compare(&case.1) == Similarity::Same);
		}

		println!("--------------------------------------------------");

		let cases = [
			"$:@capture_name(pattern)",
			"$name:@capture_name()",
			"$name:@()",
			"$name:static@capture_name(pattern)@(pattern)",
			"$name:static@(pattern)@capture_name(pattern)",
			r"$name:\@capture_name(pattern)@capture_name",
			"*",
		];

		for case in cases {
			let result = Pattern::parse(case);
			println!("case: {}", case);

			assert!(result.is_err());

			println!("\tresult: {}", result.err().unwrap());
		}
	}

	#[test]
	fn is_match() {
		struct Case<'a> {
			pattern: &'a str,
			matching: &'a [&'a str],
			nonmatching: &'a [&'a str],
		}

		let cases = [
			Case {
				pattern: "login",
				matching: &["login"],
				nonmatching: &["logout"],
			},
			Case {
				pattern: r"$id:@prefix(A|B|C)@number(\d{5})",
				matching: &["A12345", "B54321", "C11111"],
				nonmatching: &["D12345", "0ABCDEF", "AA12345", "B123456", "C1234", "AB1234"],
			},
			Case {
				pattern: r"$car:@brand(.+) (@model(.+))",
				matching: &["Audi (e-tron GT)", "Volvo (XC40 Recharge)"],
				nonmatching: &["Audi(Q8)", "Volvo C40", "Audi [A4]"],
			},
			Case {
				pattern: "*anything",
				matching: &["StarWars", "A.I."],
				nonmatching: &[],
			},
		];

		for case in cases {
			let pattern = Pattern::parse(case.pattern).unwrap();
			println!("pattern: {}", pattern);

			for text in case.matching {
				assert!(pattern.is_match(text));
			}

			for text in case.nonmatching {
				assert!(!pattern.is_match(text));
			}
		}
	}
}
