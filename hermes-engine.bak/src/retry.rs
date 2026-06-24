pub fn is_database_locked_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("database is locked") || lower.contains("database is busy")
}

pub fn lock_retry_budget() -> usize {
    std::env::var("HERMES_DB_LOCK_RETRIES")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(6)
}

pub fn lock_retry_delay_ms(attempt: usize) -> u64 {
    let clamped = attempt.min(5) as u32;
    200 * (1u64 << clamped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_database_locked_variants() {
        assert!(is_database_locked_message(
            "SqliteFailure: database is locked"
        ));
        assert!(is_database_locked_message("SQLITE database is busy"));
        assert!(!is_database_locked_message("unknown method"));
    }

    #[test]
    fn uses_default_retry_budget() {
        std::env::remove_var("HERMES_DB_LOCK_RETRIES");
        assert_eq!(lock_retry_budget(), 6);
    }

    #[test]
    fn backoff_caps_at_max_attempt() {
        assert_eq!(lock_retry_delay_ms(0), 200);
        assert_eq!(lock_retry_delay_ms(1), 400);
        assert_eq!(lock_retry_delay_ms(5), 6400);
        assert_eq!(lock_retry_delay_ms(8), 6400);
    }
}
