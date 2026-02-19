use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use chrono::Utc;
use rand::Rng;

/// Log a message to console and append to the log file.
pub fn log(level: &str, message: &str, log_file: &Path) {
    let timestamp = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let log_message = format!("[{}] [{}] {}", timestamp, level, message);
    println!("{}", log_message);

    if let Some(dir) = log_file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(log_file) {
        let _ = writeln!(f, "{}", log_message);
    }
}

/// Emit a structured event for the team visualizer.
/// Events are written as JSON files to events_dir, watched by the visualizer.
/// Best-effort: never panics or breaks the caller.
pub fn emit_event(event_type: &str, data: serde_json::Value, events_dir: &Path) {
    let _ = (|| -> Result<(), Box<dyn std::error::Error>> {
        std::fs::create_dir_all(events_dir)?;

        let mut event = data;
        if let Some(obj) = event.as_object_mut() {
            obj.insert("type".to_string(), serde_json::json!(event_type));
            obj.insert(
                "timestamp".to_string(),
                serde_json::json!(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as u64
                ),
            );
        }

        let mut rng = rand::thread_rng();
        let random_suffix: String = (0..6)
            .map(|_| {
                let idx = rng.gen_range(0..36);
                if idx < 10 {
                    (b'0' + idx) as char
                } else {
                    (b'a' + idx - 10) as char
                }
            })
            .collect();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let filename = format!("{}-{}.json", now, random_suffix);

        let content = serde_json::to_string(&event)? + "\n";
        std::fs::write(events_dir.join(filename), content)?;
        Ok(())
    })();
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_log_creates_file() {
        let tmp = TempDir::new().unwrap();
        let log_file = tmp.path().join("logs/test.log");

        log("INFO", "test message", &log_file);
        assert!(log_file.exists());

        let content = std::fs::read_to_string(&log_file).unwrap();
        assert!(content.contains("[INFO]"));
        assert!(content.contains("test message"));
    }

    #[test]
    fn test_log_appends() {
        let tmp = TempDir::new().unwrap();
        let log_file = tmp.path().join("test.log");

        log("INFO", "first", &log_file);
        log("WARN", "second", &log_file);

        let content = std::fs::read_to_string(&log_file).unwrap();
        assert!(content.contains("first"));
        assert!(content.contains("second"));
        assert_eq!(content.lines().count(), 2);
    }

    #[test]
    fn test_emit_event_creates_file() {
        let tmp = TempDir::new().unwrap();
        let events_dir = tmp.path().join("events");

        emit_event(
            "test_event",
            serde_json::json!({"key": "value"}),
            &events_dir,
        );

        assert!(events_dir.exists());
        let files: Vec<_> = std::fs::read_dir(&events_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(files.len(), 1);

        let content = std::fs::read_to_string(files[0].path()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed["type"], "test_event");
        assert_eq!(parsed["key"], "value");
        assert!(parsed["timestamp"].is_number());
    }
}
