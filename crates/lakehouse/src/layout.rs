pub const DATA_DIR: &str = "data";
pub const LOG_DIR: &str = "_log";
#[allow(dead_code)]
pub const CHECKPOINT_DIR: &str = "_checkpoints";
#[allow(dead_code)]
pub const TMPL_DIR: &str = "_tmpl";

pub fn format_log_version(version: u64) -> String {
    format!("{:020}", version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_log_version_as_20_digits() {
        assert_eq!(format_log_version(0), "00000000000000000000");
        assert_eq!(format_log_version(1), "00000000000000000001");
        assert_eq!(
            format_log_version(10000000000000000000),
            "10000000000000000000"
        );
    }
}
