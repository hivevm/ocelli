//! A ref name matched against a pattern as git's wildmatch matches one with `WM_PATHNAME`, for
//! the patterns of `merge.suppressDest`.

/// A pattern matched as git's wildmatch matches a ref with `WM_PATHNAME`: `*` and `?` stop at a
/// `/`, `**` as a whole component does not, `[...]` is a class, `[!...]` or `[^...]` its
/// complement, `\` escapes.
pub(crate) fn wildmatch(pattern: &[u8], text: &[u8]) -> bool {
    matches_from(pattern, text, true)
}

/// `wildmatch`, `component` telling whether `pattern` starts a component of the path.
fn matches_from(pattern: &[u8], text: &[u8], component: bool) -> bool {
    let wildmatch = |p: &[u8], t: &[u8]| matches_from(p, t, false);
    match pattern {
        [] => text.is_empty(),
        [b'*', b'*', rest @ ..] if component && matches!(rest, [] | [b'/', ..]) => match rest {
            [] => true,
            [_, rest @ ..] => (0..=text.len())
                .filter(|&i| i == 0 || text[i - 1] == b'/')
                .any(|i| matches_from(rest, &text[i..], true)),
        },
        [b'*', rest @ ..] => {
            let rest = rest.strip_prefix(b"*").unwrap_or(rest);
            (0..=text.len())
                .take_while(|&i| i == 0 || text[i - 1] != b'/')
                .any(|i| matches_from(rest, &text[i..], false))
        }
        [b'?', rest @ ..] => {
            matches!(text.first(), Some(c) if *c != b'/') && wildmatch(rest, &text[1..])
        }
        [b'[', rest @ ..] => {
            let Some((&c, after)) = text.split_first() else {
                return false;
            };
            let (negated, mut class) = match rest {
                [b'!' | b'^', class @ ..] => (true, class),
                class => (false, class),
            };
            let mut hit = false;
            let mut first = true;
            loop {
                if let [b'[', b':', more @ ..] = class
                    && let Some(end) = more.windows(2).position(|w| w == b":]")
                {
                    hit |= posix_class(&more[..end], c);
                    class = &more[end + 2..];
                    first = false;
                    continue;
                }
                let (lo, more) = match class {
                    [] => return false,
                    [b']', more @ ..] if !first => {
                        class = more;
                        break;
                    }
                    [b'\\', x, more @ ..] | [x, more @ ..] => (*x, more),
                };
                let (hi, more) = match more {
                    [b'-', b'\\', hi, more @ ..] => (*hi, more),
                    [b'-', hi, more @ ..] if *hi != b']' => (*hi, more),
                    _ => (lo, more),
                };
                // git takes a range's first character on its own, whatever the range.
                hit |= c == lo || (lo..=hi).contains(&c);
                class = more;
                first = false;
            }
            hit != negated && c != b'/' && wildmatch(class, after)
        }
        [b'\\', c, rest @ ..] | [c, rest @ ..] => {
            text.first() == Some(c) && matches_from(rest, &text[1..], *c == b'/')
        }
    }
}

/// Whether `c` is in the POSIX class `name`, as in `[[:digit:]]`.
fn posix_class(name: &[u8], c: u8) -> bool {
    match name {
        b"alnum" => c.is_ascii_alphanumeric(),
        b"alpha" => c.is_ascii_alphabetic(),
        b"blank" => c == b' ' || c == b'\t',
        b"cntrl" => c.is_ascii_control(),
        b"digit" => c.is_ascii_digit(),
        b"graph" => c.is_ascii_graphic(),
        b"lower" => c.is_ascii_lowercase(),
        b"print" => c.is_ascii_graphic() || c == b' ',
        b"punct" => c.is_ascii_punctuation(),
        b"space" => c.is_ascii_whitespace() || c == 0x0b,
        b"upper" => c.is_ascii_uppercase(),
        b"xdigit" => c.is_ascii_hexdigit(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::wildmatch;

    #[test]
    fn a_pattern_matches_a_branch_as_git_wildmatch_does() {
        let cases = [
            ("fe*", "feat", true),
            ("fe*", "fe/x", false),
            ("fe**", "fe/x", false),
            ("**/x", "a/b/x", true),
            ("**/x", "x", true),
            ("x/**", "x/a/b", true),
            ("rel-[0-9]", "rel-1", true),
            ("rel-[!0-9]", "rel-1", false),
            ("a[b]", "ab", true),
            ("a[]]", "a]", true),
            ("a\\*", "a*", true),
            ("a\\*", "ab", false),
            ("ma?n", "main", true),
            ("ma?n", "ma/n", false),
            ("main", "master", false),
            ("rel-[[:digit:]]", "rel-1", true),
            ("rel-[[:alpha:]]", "rel-1", false),
            ("a[a-\\z]", "ab", true),
            ("a[b-a]", "ab", true),
        ];
        for (pattern, name, matched) in cases {
            assert_eq!(
                wildmatch(pattern.as_bytes(), name.as_bytes()),
                matched,
                "{pattern} {name}"
            );
        }
    }
}
