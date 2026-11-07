use snowlib::error::FlakeError;
use snowlib::Result;

/// Builds a Nix filter expression from convenience flags.
///
/// Converts user-friendly CLI flags (--phase, --tag, etc.) into Nix expressions
/// that can be evaluated against host configurations.
///
/// # Priority order
/// The function checks parameters in this order and returns the first match:
/// 1. `explicit_filter` - If provided, returns it unchanged
/// 2. `phase` - Single phase match: `config.snowglobe.push.phase == "production"`
/// 3. `phases` - Multiple phase match (OR): `builtins.elem config.snowglobe.push.phase [...]`
/// 4. `tag` - Single tag match: `builtins.elem "web" config.snowglobe.push.tags`
/// 5. `tags` - Multiple tag match (OR): `builtins.any (tag: builtins.elem tag config.snowglobe.push.tags) [...]`
/// 6. None - No filter specified
///
/// # Arguments
/// * `explicit_filter` - Raw Nix expression from `-e/--filter` flag
/// * `phase` - Single phase name from `--phase` flag
/// * `phases` - Multiple phase names from `--phases` flag (comma-separated)
/// * `tag` - Single tag name from `--tag` flag
/// * `tags` - Multiple tag names from `--tags` flag (comma-separated)
///
/// # Returns
/// * `Ok(Some(expr))` - A Nix expression string ready for evaluation
/// * `Ok(None)` - No filter specified (all parameters are None/empty)
/// * `Err` - Validation failed (invalid characters, empty names after trimming, etc.)
///
/// # Examples
/// ```ignore
/// // Single phase filter
/// let expr = build_filter_expression(None, Some("production"), &[], None, &[])?;
/// // Returns: Some("config: (config.snowglobe.push.phase or null) == \"production\"")
///
/// // Multiple tags filter
/// let expr = build_filter_expression(None, None, &[], None, &["web".to_string(), "api".to_string()])?;
/// // Returns: Some("config: builtins.any (tag: builtins.elem tag (config.snowglobe.push.tags or [])) [\"web\" \"api\"]")
/// ```
pub fn build_filter_expression(
    explicit_filter: Option<&str>,
    phase: Option<&str>,
    phases: &[String],
    tag: Option<&str>,
    tags: &[String],
) -> Result<Option<String>> {
    if let Some(expr) = explicit_filter {
        Ok(Some(expr.to_string()))
    } else if let Some(phase) = phase {
        let sanitized = validate_nix_identifier(phase.trim())?;
        Ok(Some(format!(
            r#"config: (config.snowglobe.push.phase or null) == "{}""#,
            sanitized
        )))
    } else if !phases.is_empty() {
        let phase_list = phases
            .iter()
            .map(|p| validate_nix_identifier(p.trim()))
            .collect::<Result<Vec<_>>>()?
            .iter()
            .map(|p| format!(r#""{}""#, p))
            .collect::<Vec<_>>()
            .join(" ");
        Ok(Some(format!(
            "config: builtins.elem (config.snowglobe.push.phase or null) [{}]",
            phase_list
        )))
    } else if let Some(tag) = tag {
        let sanitized = validate_nix_identifier(tag.trim())?;
        Ok(Some(format!(
            r#"config: builtins.elem "{}" (config.snowglobe.push.tags or [])"#,
            sanitized
        )))
    } else if !tags.is_empty() {
        let tag_list = tags
            .iter()
            .map(|t| validate_nix_identifier(t.trim()))
            .collect::<Result<Vec<_>>>()?
            .iter()
            .map(|t| format!(r#""{}""#, t))
            .collect::<Vec<_>>()
            .join(" ");
        Ok(Some(format!(
            "config: builtins.any (tag: builtins.elem tag (config.snowglobe.push.tags or [])) [{}]",
            tag_list
        )))
    } else {
        Ok(None)
    }
}

/// Validates that a phase or tag name contains only allowed characters.
///
/// Valid characters: ASCII alphanumeric, dash (-), and underscore (_)
///
/// # Examples
/// - Valid: "production", "staging-1", "web_servers", "dev"
/// - Invalid: "prod.env" (contains period), "test env" (contains space), "日本" (non-ASCII)
fn validate_nix_identifier(s: &str) -> Result<String> {
    let sanitized: String = s
        .chars()
        .filter(|c| snowlib::push::is_name_char(*c))
        .collect();

    if sanitized.is_empty() {
        return Err(FlakeError::UnusableFilterName {
            name: s.to_string(),
        }
        .into());
    }
    if sanitized != s {
        return Err(FlakeError::InvalidFilterName {
            name: s.to_string(),
            suggestion: sanitized,
        }
        .into());
    }

    Ok(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_filter_expression_single_phase() {
        let result = build_filter_expression(None, Some("production"), &[], None, &[]).unwrap();
        assert_eq!(
            result,
            Some(r#"config: (config.snowglobe.push.phase or null) == "production""#.to_string())
        );
    }

    #[test]
    fn test_build_filter_expression_multiple_phases() {
        let result = build_filter_expression(
            None,
            None,
            &["production".to_string(), "staging".to_string()],
            None,
            &[],
        )
        .unwrap();
        assert_eq!(
            result,
            Some(
                r#"config: builtins.elem (config.snowglobe.push.phase or null) ["production" "staging"]"#
                    .to_string()
            )
        );
    }

    #[test]
    fn test_build_filter_expression_single_tag() {
        let result = build_filter_expression(None, None, &[], Some("web"), &[]).unwrap();
        assert_eq!(
            result,
            Some(r#"config: builtins.elem "web" (config.snowglobe.push.tags or [])"#.to_string())
        );
    }

    #[test]
    fn test_build_filter_expression_multiple_tags() {
        let result = build_filter_expression(
            None,
            None,
            &[],
            None,
            &["web".to_string(), "frontend".to_string()],
        )
        .unwrap();
        assert_eq!(
            result,
            Some(
                r#"config: builtins.any (tag: builtins.elem tag (config.snowglobe.push.tags or [])) ["web" "frontend"]"#
                    .to_string()
            )
        );
    }

    #[test]
    fn test_build_filter_expression_explicit_filter() {
        let result = build_filter_expression(
            Some("config: config.services.nginx.enable"),
            None,
            &[],
            None,
            &[],
        )
        .unwrap();
        assert_eq!(
            result,
            Some("config: config.services.nginx.enable".to_string())
        );
    }

    #[test]
    fn test_build_filter_expression_none() {
        let result = build_filter_expression(None, None, &[], None, &[]).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_build_filter_expression_with_whitespace() {
        let result = build_filter_expression(
            None,
            None,
            &["production".to_string(), " staging ".to_string()],
            None,
            &[],
        )
        .unwrap();
        assert_eq!(
            result,
            Some(
                r#"config: builtins.elem (config.snowglobe.push.phase or null) ["production" "staging"]"#
                    .to_string()
            )
        );
    }

    #[test]
    fn test_validate_nix_identifier_valid() {
        assert_eq!(validate_nix_identifier("production").unwrap(), "production");
        assert_eq!(validate_nix_identifier("test-env").unwrap(), "test-env");
        assert_eq!(validate_nix_identifier("test_env").unwrap(), "test_env");
        assert_eq!(validate_nix_identifier("env123").unwrap(), "env123");
        assert_eq!(validate_nix_identifier("123").unwrap(), "123");
    }

    fn suggestion_for(name: &str) -> String {
        match validate_nix_identifier(name) {
            Err(snowlib::error::SnowError::Flake(FlakeError::InvalidFilterName {
                suggestion,
                ..
            })) => suggestion,
            other => panic!("expected InvalidFilterName for {name:?}, got {other:?}"),
        }
    }

    fn assert_unusable(name: &str) {
        assert!(
            matches!(
                validate_nix_identifier(name),
                Err(snowlib::error::SnowError::Flake(
                    FlakeError::UnusableFilterName { .. }
                ))
            ),
            "expected UnusableFilterName for {name:?}"
        );
    }

    #[test]
    fn test_validate_nix_identifier_invalid_chars_suggests_the_allowed_ones() {
        assert_eq!(suggestion_for("test env"), "testenv");
        assert_eq!(suggestion_for("test@env"), "testenv");
        assert_eq!(suggestion_for("web.server"), "webserver");
        assert_eq!(suggestion_for("prod-日本"), "prod-");
    }

    #[test]
    fn test_validate_nix_identifier_without_allowed_chars_is_unusable() {
        assert_unusable("");
        assert_unusable("!@#$");
        assert_unusable("...");
        assert_unusable("日本語");
    }

    fn message_and_help(name: &str) -> (String, String) {
        let error = validate_nix_identifier(name).unwrap_err();
        let help = miette::Diagnostic::help(&error).unwrap().to_string();
        (error.to_string(), help)
    }

    #[test]
    fn test_invalid_filter_name_message_and_help() {
        assert_eq!(
            message_and_help("prod!uction"),
            (
                "Invalid phase or tag name 'prod!uction'".to_string(),
                "Use only ASCII alphanumeric characters, dashes and underscores: try 'production'."
                    .to_string()
            )
        );
    }

    #[test]
    fn test_unusable_filter_name_message_and_help() {
        assert_eq!(
            message_and_help("!@#$"),
            (
                "Invalid phase or tag name '!@#$'".to_string(),
                "A phase or tag name must contain at least one ASCII alphanumeric character, dash or underscore, as in 'production' or 'web_servers'."
                    .to_string()
            )
        );
    }

    #[test]
    fn test_build_filter_expression_trim_phase() {
        let result = build_filter_expression(None, Some(" production "), &[], None, &[]).unwrap();
        assert_eq!(
            result,
            Some(r#"config: (config.snowglobe.push.phase or null) == "production""#.to_string())
        );
    }

    #[test]
    fn test_build_filter_expression_trim_tag() {
        let result = build_filter_expression(None, None, &[], Some(" web "), &[]).unwrap();
        assert_eq!(
            result,
            Some(r#"config: builtins.elem "web" (config.snowglobe.push.tags or [])"#.to_string())
        );
    }
}
