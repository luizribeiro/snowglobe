/// `one` or `many`, whichever fits `count`
pub fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 {
        one
    } else {
        many
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plural_picks_the_form_for_the_count() {
        assert_eq!(plural(0, "fetch", "fetches"), "fetches");
        assert_eq!(plural(1, "fetch", "fetches"), "fetch");
        assert_eq!(plural(2, "fetch", "fetches"), "fetches");
    }
}
