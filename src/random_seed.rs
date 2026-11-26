/// Reads an unsigned 64-bit seed from the given environment variable, falling
/// back to a freshly generated one if it is unset or invalid.
pub(crate) fn from_env_or_random(var: &str) -> u64 {
    std::env::var(var)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| {
            use std::hash::{BuildHasher, Hasher};
            std::collections::hash_map::RandomState::new()
                .build_hasher()
                .finish()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_var_wins_when_present_and_valid() {
        std::env::set_var("TAPLINE_RANDOM_SEED_TEST_VAR", "42");
        assert_eq!(from_env_or_random("TAPLINE_RANDOM_SEED_TEST_VAR"), 42);
        std::env::remove_var("TAPLINE_RANDOM_SEED_TEST_VAR");
    }

    #[test]
    fn falls_back_to_a_generated_seed_when_unset_or_invalid() {
        std::env::remove_var("TAPLINE_RANDOM_SEED_TEST_VAR_2");
        let _ = from_env_or_random("TAPLINE_RANDOM_SEED_TEST_VAR_2");
        std::env::set_var("TAPLINE_RANDOM_SEED_TEST_VAR_2", "not-a-number");
        let _ = from_env_or_random("TAPLINE_RANDOM_SEED_TEST_VAR_2");
        std::env::remove_var("TAPLINE_RANDOM_SEED_TEST_VAR_2");
    }
}
