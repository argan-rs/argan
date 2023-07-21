use std::{
	collections::HashMap,
	fmt::{format, Display},
	iter::Peekable,
	str::Chars,
	sync::Arc,
};

use regex::Regex;

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
			chars.next();

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
					Segment::Static(pattern) => {
						panic!("regex pattern must have at least one capturing segment")
					}
					Segment::Capturing {
						some_name,
						subpattern,
					} => {
						let regex_pattern = if let Some(capture_name) = some_name {
							format!("\\A(?P<{}>{})\\z", capture_name, subpattern)
						} else {
							format!("\\A({})\\z", subpattern)
						};

						let regex = Regex::new(&regex_pattern).unwrap();

						return Pattern::Regex(name.into(), Some(regex));
					}
				};
			}

			let mut regex_pattern = "\\A".to_owned();

			let mut capturing_segment_without_name = false;
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
							if capturing_segment_without_name {
								panic!(
									"regex pattern without a capture name cannot have multiple capturing segments",
								)
							}

							format!("(?P<{}>{})", capture_name, subpattern)
						} else {
							if capturing_segments_count > 0 {
								panic!("regex pattern with multiple capturing segments cannot omit a capture name")
							}

							capturing_segment_without_name = true;
							format!("({})", subpattern)
						};

						capturing_segments_count += 1;
						regex_pattern.push_str(&regex_subpattern);
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

	pub fn is_match(&self, string: &str) -> bool {
		match self {
			Pattern::Static(pattern) => pattern.as_ref() == string,
			Pattern::Regex(_, some_regex) => {
				if let Some(regex) = some_regex {
					regex.is_match(string)
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
					if some_regex.as_ref().is_some_and(|regex| {
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
				panic!("incomplete pattern")
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

// Returns a regex subpattern if the end of the regex segment is found. Otherwise None.
// Regex pattern maybe empty if the end of the regex segment is met right away.
fn split_off_subpattern(chars: &mut Peekable<Chars<'_>>) -> Option<String> {
	let mut subpattern = String::new();
	let mut depth = 1; // We are already inside the opened '(' bracket.
	let mut unescaped = true;
	let mut in_character_class = -1i8;

	while let Some(ch) = chars.next() {
		if ch == ')' && (unescaped || in_character_class < 0) {
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
				if unescaped || in_character_class > -1 {
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
