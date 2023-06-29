use std::{collections::HashMap, fmt::format, iter::Peekable, str::CharIndices};

use regex::Regex;

// --------------------------------------------------

pub struct Matcher(Pattern);

enum Pattern {
	Static(Option<String>, String),   // Resource name is optional.
	Regex(String, Regex),             // Resource name is mandatory.
	Wildcard(Option<String>, String), // Resource name is optional.
}

impl Matcher {
	pub fn new(pattern: &str) -> Matcher {
		let (some_name, mut slices) = parse(pattern);
		if slices.is_empty() {
			panic!("incomplete pattern")
		}

		if slices.len() == 1 {
			let pattern = match slices.pop().unwrap() {
				Slice::Static(pattern) => Pattern::Static(some_name, pattern),
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
						Pattern::Wildcard(some_name, name)
					}
				}
			};

			return Matcher(pattern);
		}

		let Some(pattern_name) = some_name else {
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

		Matcher(Pattern::Regex(pattern_name, regex))
	}

	pub fn is_match(&self, pattern: &str) -> (bool, HashMap<&'static str, String>) {
		todo!()
	}
}

enum Slice {
	Static(String),
	Regex {
		name: String,
		some_pattern: Option<String>,
	},
}

fn parse(pattern: &str) -> (Option<String>, Vec<Slice>) {
	if pattern.is_empty() {
		panic!("empty pattern")
	}

	let mut chars = pattern.char_indices().peekable();

	let (some_name, some_delimiter) = if let Some((_, '$')) = chars.peek() {
		chars.next();

		let (name, some_delimiter) = split_at_delimiter(&mut chars, |ch| ch == ':');
		if name.is_empty() {
			panic!("empty pattern name")
		}

		if some_delimiter.is_none() {
			panic!("incomplete pattern")
		}

		(Some(name), some_delimiter)
	} else {
		(None, None)
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
			let (name, some_delimiter) = split_at_delimiter(&mut chars, |ch| ch == ':' || ch == '}');
			if name.is_empty() {
				panic!("empty regex slice name")
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
				name,
				some_pattern: some_regex,
			});
			parsing_static = true;
		}
	}

	(some_name, slices)
}

// Returns the segment before the delimiter and the delimiter. If the delimiter
// is not found then the segment contains all the chars and the returned delimiter
// will be None. If there are no more chars or the delimiter is found right away
// then the returned segment will be empty.
fn split_at_delimiter(
	chars: &mut Peekable<CharIndices<'_>>,
	delimiter: impl Fn(char) -> bool,
) -> (String, Option<char>) {
	let mut buf = String::new();
	let mut unescaped = true;

	while let Some((_, ch)) = chars.next() {
		if delimiter(ch) {
			if unescaped {
				return (buf, Some(ch));
			}

			unescaped = true;
		} else if ch == '\\' {
			if let Some((_, next_ch)) = chars.peek() {
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
fn split_regex(chars: &mut Peekable<CharIndices<'_>>) -> Option<String> {
	let mut regex = String::new();
	let mut depth = 1; // We are already inside the opened '{' bracket.
	let mut unescaped = true;
	let mut in_character_class = -1i8;

	while let Some((_, ch)) = chars.next() {
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
					if let Some((_, '\\' | '[' | ']' | '{' | '}')) = chars.peek() {
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
					if let Some((_, ']')) = chars.peek() {
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
