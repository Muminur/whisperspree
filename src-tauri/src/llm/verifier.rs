use crate::error::Error;

/// Enforces all three §7.4 post-call invariants before injection.
pub fn verify(input: &str, output: &str) -> Result<(), Error> {
    if output.trim().is_empty() || output.len() > input.len().saturating_mul(4) {
        return Err(Error::LlmVerify(
            "LLM output was empty or exceeded the runaway guard".into(),
        ));
    }
    let placeholders = placeholder_re()
        .find_iter(input)
        .map(|m| m.as_str())
        .collect::<Vec<_>>();
    for token in placeholders {
        if placeholder_re()
            .find_iter(output)
            .filter(|candidate| candidate.as_str() == token)
            .count()
            != 1
        {
            return Err(Error::LlmVerify(
                "LLM output did not preserve snippet placeholders".into(),
            ));
        }
    }
    Ok(())
}

fn placeholder_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"⟦S[^⟧]*⟧").expect("static placeholder regex"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_7_4_accepts_nonempty_bounded_output_with_each_placeholder_once() {
        assert!(verify("put ⟦S0⟧ then ⟦S1⟧", "Put ⟦S0⟧ then ⟦S1⟧.").is_ok());
    }

    #[test]
    fn fr_7_4_rejects_missing_duplicate_empty_and_runaway_outputs() {
        for output in ["", "missing", "⟦S0⟧ ⟦S0⟧", &"x".repeat(17)] {
            assert!(
                verify("abc ⟦S0⟧", output).is_err(),
                "must reject {output:?}"
            );
        }
    }
}
