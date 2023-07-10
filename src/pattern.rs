use std::{iter::Peekable, str::Chars};

use regex::Regex;

// --------------------------------------------------
// static:		static, resource
// regex:			$name:{capture_name:pattern}escaped{capture_2}escaped{capture_name:pattern}
// wildcard:	$name

pub(crate) enum Pattern {
	Static(String),
	Regex(String, Regex),
	Wildcard(String),
}

impl Pattern {
	pub fn parse(pattern: &str) -> Pattern {
		let (some_name, mut some_slices) = split(pattern);

		if let Some(mut slices) = some_slices {
			if slices.len() == 1 {
				let pattern = match slices.pop().unwrap() {
					Slice::Static(pattern) => Pattern::Static(pattern),
					Slice::Regex { name, some_pattern } => {
						if let Some(mut regex_pattern) = some_pattern {
							regex_pattern = format!("\\A(?P<{}>{})\\z", name, regex_pattern);
							let pattern_name = if let Some(pattern_name) = some_name {
								pattern_name
							} else {
								name
							};

							let regex = Regex::new(&regex_pattern).unwrap();

							Pattern::Regex(pattern_name, regex)
						} else {
							panic!("single 'match all' regex pattern is equivalent to wildcard and not allowed",)
						}
					}
				};

				return pattern;
			}

			let Some(name) = some_name else {
				panic!("no regex pattern name")
			};

			let mut regex_pattern = "\\A".to_owned();

			for slice in slices {
				match slice {
					Slice::Static(mut pattern) => {
						pattern = regex::escape(&pattern);
						regex_pattern.push_str(&pattern);
					}
					Slice::Regex { name, some_pattern } => {
						let pattern = if let Some(pattern) = some_pattern {
							format!("(?P<{}>{})", name, pattern)
						} else {
							format!("(?P<{}>{})", name, "(?s).*")
						};

						regex_pattern.push_str(&pattern);
					}
				}
			}

			regex_pattern.push_str("\\z");
			let regex = Regex::new(&regex_pattern).unwrap();

			return Pattern::Regex(name, regex);
		}

		if let Some(name) = some_name {
			return Pattern::Wildcard(name);
		}

		panic!("incomplete pattern")
	}

	pub fn name(&self) -> Option<&str> {
		match self {
			Pattern::Static(_) => None,
			Pattern::Regex(name, _) => Some(name.as_str()),
			Pattern::Wildcard(name) => Some(name.as_str()),
		}
	}

	pub fn is_match(&self, string: &str) -> bool {
		match self {
			Pattern::Static(pattern) => pattern == string,
			Pattern::Regex(_, regex) => regex.is_match(string),
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
			Pattern::Regex(name, regex) => {
				if let Pattern::Regex(other_some_name, other_regex) = other {
					if regex.as_str() == other_regex.as_str() {
						if name == other_some_name {
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

		return Similarity::Different;
	}
}

enum Slice {
	Static(String),
	Regex {
		name: String,
		some_pattern: Option<String>,
	},
}

fn split(pattern: &str) -> (Option<String>, Option<Vec<Slice>>) {
	if pattern.is_empty() {
		panic!("empty pattern")
	}

	let mut chars = pattern.chars().peekable();

	let some_name = if let Some('$') = chars.peek() {
		chars.next();

		let (name, some_delimiter) = split_at_delimiter(&mut chars, |ch| ch == ':');
		if name.is_empty() {
			panic!("empty pattern name")
		}

		if some_delimiter.is_none() {
			return (Some(name), None);
		}

		Some(name)
	} else {
		None
	};

	let mut slices = Vec::new();
	let mut parsing_static = true;

	loop {
		if parsing_static {
			let (static_segment, some_delimiter) = split_at_delimiter(&mut chars, |ch| ch == '{');
			if !static_segment.is_empty() {
				slices.push(Slice::Static(static_segment));
			}

			if some_delimiter.is_some() {
				parsing_static = false
			} else {
				break;
			}
		} else {
			let (capture_name, some_delimiter) =
				split_at_delimiter(&mut chars, |ch| ch == ':' || ch == '}');
			if capture_name.is_empty() {
				panic!("empty regex capture name")
			};

			let Some(delimiter) = some_delimiter else {
				panic!("incomplete pattern")
			};

			let some_regex = if delimiter == ':' {
				let Some(regex) = split_regex(&mut chars) else {
					panic!("incomplete pattern")
				};

				if regex.is_empty() {
					panic!("empty regex slice pattern")
				}

				Some(regex)
			} else {
				None
			};

			slices.push(Slice::Regex {
				name: capture_name,
				some_pattern: some_regex,
			});
			parsing_static = true;
		}
	}

	(some_name, Some(slices))
}

// Returns the segment before the delimiter and the delimiter. If the delimiter
// is not found then the segment contains all the chars and the returned delimiter
// will be None. If there are no more chars or the delimiter is found right away
// then the returned segment will be empty.
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

// Returns a regex pattern if the end of the regex segment is found. Otherwise None.
// Regex pattern maybe empty if the end of the regex segment is met right away.
fn split_regex(chars: &mut Peekable<Chars<'_>>) -> Option<String> {
	let mut regex = String::new();
	let mut depth = 1; // We are already inside the opened '{' bracket.
	let mut unescaped = true;
	let mut in_character_class = -1i8;

	while let Some(ch) = chars.next() {
		if ch == '}' && (unescaped || in_character_class < 0) {
			depth -= 1;
			if depth == 0 {
				return Some(regex);
			}

			regex.push(ch);

			continue;
		}

		regex.push(ch);

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
					if let Some('\\' | '[' | ']' | '{' | '}') = chars.peek() {
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
