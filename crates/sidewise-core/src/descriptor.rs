/// Lightweight HID report descriptor reader for cross-platform parsing and replay tests.

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedControl {
    pub usage_page: u16,
    pub usage: u16,
    pub is_button: bool,
    pub logical_min: i32,
    pub logical_max: i32,
    pub bit_size: u32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParsedDescriptor {
    pub controls: Vec<ParsedControl>,
    pub has_confidence: bool,
    pub x_min: i32,
    pub x_max: i32,
    pub y_min: i32,
    pub y_max: i32,
}

pub fn parse_report_descriptor(bytes: &[u8]) -> ParsedDescriptor {
    let mut controls = Vec::new();
    let mut i = 0;

    let mut current_page: u16 = 0;
    let mut current_usages: Vec<u16> = Vec::new();
    let mut log_min: i32 = 0;
    let mut log_max: i32 = 0;
    let mut report_size: u32 = 0;
    let mut report_count: u32 = 0;

    let mut x_min = 0;
    let mut x_max = 4096;
    let mut y_min = 0;
    let mut y_max = 4096;
    let mut has_confidence = false;

    while i < bytes.len() {
        let b = bytes[i];
        if b == 0xFE {
            // Long item (rare in HID, skip)
            if i + 2 < bytes.len() {
                let data_size = bytes[i + 1] as usize;
                i += 3 + data_size;
            } else {
                break;
            }
            continue;
        }

        let b_size_code = b & 0x03;
        let data_size = match b_size_code {
            3 => 4,
            s => s as usize,
        };
        let b_type = (b >> 2) & 0x03;
        let b_tag = (b >> 4) & 0x0F;

        let mut data: u32 = 0;
        for j in 0..data_size {
            if i + 1 + j < bytes.len() {
                data |= (bytes[i + 1 + j] as u32) << (j * 8);
            }
        }
        let signed_data: i32 = match data_size {
            1 => (data as u8) as i8 as i32,
            2 => (data as u16) as i16 as i32,
            4 => data as i32,
            _ => 0,
        };

        i += 1 + data_size;

        match (b_type, b_tag) {
            // Global Items
            (1, 0) => current_page = data as u16,     // Usage Page
            (1, 1) => log_min = signed_data,          // Logical Minimum
            (1, 2) => log_max = signed_data,          // Logical Maximum
            (1, 7) => report_size = data,             // Report Size
            (1, 9) => report_count = data,            // Report Count

            // Local Items
            (2, 0) => current_usages.push(data as u16), // Usage
            (2, 1) => {                                 // Usage Minimum
                // In range
                current_usages.push(data as u16);
            }
            (2, 2) => {                                 // Usage Maximum
                // Can track range if needed
            }

            // Main Items
            (0, 8) => {
                // Input item
                // A control with 1 bit and log_max == 1 is typically a button / boolean flag
                let is_button = report_size == 1 && log_min == 0 && log_max == 1;

                for &usage in &current_usages {
                    if current_page == 0x0D && usage == 0x47 {
                        has_confidence = true;
                    }
                    if current_page == 0x01 && usage == 0x30 {
                        x_min = log_min;
                        x_max = log_max;
                    }
                    if current_page == 0x01 && usage == 0x31 {
                        y_min = log_min;
                        y_max = log_max;
                    }

                    controls.push(ParsedControl {
                        usage_page: current_page,
                        usage,
                        is_button,
                        logical_min: log_min,
                        logical_max: log_max,
                        bit_size: report_size * report_count.max(1),
                    });
                }
                current_usages.clear();
            }
            (0, 10) => {
                // Collection
                current_usages.clear();
            }
            (0, 12) => {
                // End Collection
                current_usages.clear();
            }
            _ => {}
        }
    }

    ParsedDescriptor {
        controls,
        has_confidence,
        x_min,
        x_max,
        y_min,
        y_max,
    }
}
