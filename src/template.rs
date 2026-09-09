// SPDX-License-Identifier: Apache-2.0
//! Literal substitution for compiler-owned fragments, not a template language.
//! Values are opaque: an inserted value is never parsed or expanded again.

pub(crate) fn render(template: &str, values: &[(&str, &str)]) -> String {
    for (index, (name, _)) in values.iter().enumerate() {
        assert!(
            !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_'),
            "invalid template parameter {name:?}"
        );
        assert!(
            !values[..index].iter().any(|(other, _)| other == name),
            "duplicate template parameter {name}"
        );
    }
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('@') {
        out.push_str(&rest[..start]);
        rest = &rest[start + 1..];
        let end = rest.find('@').expect("unterminated template parameter");
        let name = &rest[..end];
        if name.is_empty() {
            out.push('@'); // @@ is a literal @ in a compiler-owned template.
        } else {
            let (_, value) = values
                .iter()
                .find(|(key, _)| *key == name)
                .unwrap_or_else(|| panic!("unbound template parameter {name}"));
            out.push_str(value);
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::render;

    #[test]
    fn substitutions_are_literal_and_order_independent() {
        let values = [("A", "@B@"), ("B", "雪")];
        assert_eq!(
            render("@A@ / @B@ / @@ / @A@", &values),
            "@B@ / 雪 / @ / @B@"
        );
        assert_eq!(
            render("@A@ / @B@", &values),
            render("@A@ / @B@", &[values[1], values[0]])
        );
        assert_eq!(render("", &values), "");
    }

    #[test]
    #[should_panic(expected = "unbound template parameter TYPO")]
    fn missing_parameters_fail_at_generation_time() {
        render("@TYPO@", &[("NAME", "value")]);
    }

    #[test]
    #[should_panic(expected = "duplicate template parameter NAME")]
    fn duplicate_parameters_are_rejected() {
        render("@NAME@", &[("NAME", "a"), ("NAME", "b")]);
    }

    #[test]
    #[should_panic(expected = "invalid template parameter")]
    fn invalid_parameter_names_are_rejected() {
        render("literal", &[("name", "value")]);
    }

    #[test]
    #[should_panic(expected = "unterminated template parameter")]
    fn unterminated_parameters_are_rejected() {
        render("@NAME", &[("NAME", "a")]);
    }
}
