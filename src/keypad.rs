use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct Keypad {
    row0: u8,
    row1: u8,
    data: u8,
    pub interrupt: u8,
}

#[derive(Copy, Clone, Serialize, Deserialize)]
pub enum KeypadKey {
    Right,
    Left,
    Up,
    Down,
    A,
    B,
    Select,
    Start,
}

impl Keypad {
    pub fn new() -> Keypad {
        Keypad {
            row0: 0x0F,
            row1: 0x0F,
            data: 0xFF,
            interrupt: 0,
        }
    }

    pub fn rb(&self) -> u8 {
        self.data
    }

    pub fn wb(&mut self, value: u8) {
        self.data = (self.data & 0xCF) | (value & 0x30);
        self.update();
    }

        /// Set keypad state from an 8-bit mask.
    /// Bit mapping (mask bit = 1 means pressed):
    ///  bit0 = Right
    ///  bit1 = Left
    ///  bit2 = Up
    ///  bit3 = Down
    ///  bit4 = A
    ///  bit5 = B
    ///  bit6 = Select
    ///  bit7 = Start
    pub fn set_mask(&mut self, mask: u8) {
        // Lower nibble -> directions (row0)
        let dir_mask = mask & 0x0F;
        // Upper nibble -> buttons (row1)
        let btn_mask = (mask >> 4) & 0x0F;

        // In this Keypad implementation 0 = pressed, 1 = released for bits within row0/row1
        // So compute row values by starting with all 1s (0x0F) and clearing bits for pressed keys.
        self.row0 = 0x0F & !(dir_mask & 0x0F);
        self.row1 = 0x0F & !(btn_mask & 0x0F);

        // Keep the high nibble of data (bits 4/5 define selection) — we don't change it here.
        // Call update() to refresh self.data and interrupt flags.
        self.update();
    }

    fn update(&mut self) {
        let old_values = self.data & 0xF;
        let mut new_values = 0xF;

        if self.data & 0x10 == 0x00 {
            new_values &= self.row0;
        }
        if self.data & 0x20 == 0x00 {
            new_values &= self.row1;
        }

        if old_values == 0xF && new_values != 0xF {
            self.interrupt |= 0x10;
        }

        self.data = (self.data & 0xF0) | new_values;
    }

    pub fn keydown(&mut self, key: KeypadKey) {
        match key {
            KeypadKey::Right => self.row0 &= !(1 << 0),
            KeypadKey::Left => self.row0 &= !(1 << 1),
            KeypadKey::Up => self.row0 &= !(1 << 2),
            KeypadKey::Down => self.row0 &= !(1 << 3),
            KeypadKey::A => self.row1 &= !(1 << 0),
            KeypadKey::B => self.row1 &= !(1 << 1),
            KeypadKey::Select => self.row1 &= !(1 << 2),
            KeypadKey::Start => self.row1 &= !(1 << 3),
        }
        self.update();
    }

    pub fn keyup(&mut self, key: KeypadKey) {
        match key {
            KeypadKey::Right => self.row0 |= 1 << 0,
            KeypadKey::Left => self.row0 |= 1 << 1,
            KeypadKey::Up => self.row0 |= 1 << 2,
            KeypadKey::Down => self.row0 |= 1 << 3,
            KeypadKey::A => self.row1 |= 1 << 0,
            KeypadKey::B => self.row1 |= 1 << 1,
            KeypadKey::Select => self.row1 |= 1 << 2,
            KeypadKey::Start => self.row1 |= 1 << 3,
        }
        self.update();
    }
}

#[cfg(test)]
mod test {
    use super::KeypadKey;

    #[test]
    fn keys_buttons() {
        let mut keypad = super::Keypad::new();
        let keys0: [KeypadKey; 4] = [
            KeypadKey::A,
            KeypadKey::B,
            KeypadKey::Select,
            KeypadKey::Start,
        ];

        for i in 0..keys0.len() {
            keypad.keydown(keys0[i]);

            keypad.wb(0x00);
            assert_eq!(keypad.rb(), 0xCF & !(1 << i));

            keypad.wb(0x10);
            assert_eq!(keypad.rb(), 0xDF & !(1 << i));

            keypad.wb(0x20);
            assert_eq!(keypad.rb(), 0xEF);

            keypad.wb(0x30);
            assert_eq!(keypad.rb(), 0xFF);

            keypad.keyup(keys0[i]);
        }
    }

    #[test]
    fn keys_direction() {
        let mut keypad = super::Keypad::new();
        let keys1: [KeypadKey; 4] = [
            KeypadKey::Right,
            KeypadKey::Left,
            KeypadKey::Up,
            KeypadKey::Down,
        ];

        for i in 0..keys1.len() {
            keypad.keydown(keys1[i]);

            keypad.wb(0x00);
            assert_eq!(keypad.rb(), 0xCF & !(1 << i));

            keypad.wb(0x10);
            assert_eq!(keypad.rb(), 0xDF);

            keypad.wb(0x20);
            assert_eq!(keypad.rb(), 0xEF & !(1 << i));

            keypad.wb(0x30);
            assert_eq!(keypad.rb(), 0xFF);

            keypad.keyup(keys1[i]);
        }
    }
}
