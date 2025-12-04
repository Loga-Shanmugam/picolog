use picologv3::Logger;
use std::fs;

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
struct TestData {
    id: u64,
    val: u32,
}

impl Default for TestData {
    fn default() -> Self {
        Self { id: 0, val: 0 }
    }
}

#[test]
fn test_file_integrity() {
    let path = "integrity_test.log";
    if std::path::Path::new(path).exists() {
        fs::remove_file(path).unwrap();
    }

    // Write
    {
        let mut logger = Logger::<TestData>::new()
            .with_write_config(path.to_string(), 1024, 1_000_000, 100_000); // 1ms flush
        logger.start().unwrap();

        for i in 0..100 {
            logger.log(TestData {
                id: i as u64,
                val: (i * 10) as u32,
            });
        }
        // Drop logger to flush and close
    }

    let logger = Logger::<TestData>::new().with_read_config(path.to_string());
    let result = logger.read().unwrap();

    assert_eq!(result.len(), 100, "Should have read 100 items");

    for (i, item) in result.iter().enumerate() {
        assert_eq!(item.id, i as u64, "ID mismatch at index {}", i);
        assert_eq!(item.val, (i * 10) as u32, "Value mismatch at index {}", i);
    }

    if std::path::Path::new(path).exists() {
        fs::remove_file(path).unwrap();
    }
}
