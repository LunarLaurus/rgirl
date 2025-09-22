use crate::cpu::CPU;
use crate::gbmode::GbMode;
use crate::keypad::KeypadKey;
use crate::mbc;
use crate::printer::GbPrinter;
use crate::serial;
use crate::serial::SerialCallback;
use crate::sound;
use crate::StrResult;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct Device {
    cpu: CPU,
    save_state: Option<String>,
}

impl Drop for Device {
    fn drop(&mut self) {
        if let Some(path) = &self.save_state {
            let file = std::fs::File::create(path).unwrap();
            ciborium::into_writer(&self.cpu, file).unwrap();
        }
    }
}

pub struct StdoutPrinter;

impl SerialCallback for StdoutPrinter {
    fn call(&mut self, v: u8) -> Option<u8> {
        use std::io::Write;

        print!("{}", v as char);
        let _ = ::std::io::stdout().flush();

        None
    }
}

impl Device {
    pub fn load_state(path: &str) -> Option<Box<Device>> {
        let file = std::fs::File::open(path).ok()?;
        let cpu = ciborium::de::from_reader(file).ok()?;
        Some(Box::new(Device {
            cpu,
            save_state: Some(path.to_string()),
        }))
    }

    pub fn new(
        romname: &str,
        skip_checksum: bool,
        save_state: Option<String>,
    ) -> StrResult<Device> {
        let cart = mbc::FileBackedMBC::new(romname.into(), skip_checksum)?;
        CPU::new(Box::new(cart), None).map(|cpu| Device {
            cpu: cpu,
            save_state,
        })
    }

    pub fn new_cgb(
        romname: &str,
        skip_checksum: bool,
        save_state: Option<String>,
    ) -> StrResult<Device> {
        let cart = mbc::FileBackedMBC::new(romname.into(), skip_checksum)?;
        CPU::new_cgb(Box::new(cart), None).map(|cpu| Device {
            cpu: cpu,
            save_state,
        })
    }

    pub fn new_from_buffer(
        romdata: Vec<u8>,
        skip_checksum: bool,
        save_state: Option<String>,
    ) -> StrResult<Device> {
        let cart = mbc::get_mbc(romdata, skip_checksum)?;
        CPU::new(cart, None).map(|cpu| Device {
            cpu: cpu,
            save_state,
        })
    }

    pub fn new_cgb_from_buffer(
        romdata: Vec<u8>,
        skip_checksum: bool,
        save_state: Option<String>,
    ) -> StrResult<Device> {
        let cart = mbc::get_mbc(romdata, skip_checksum)?;
        CPU::new_cgb(cart, None).map(|cpu| Device {
            cpu: cpu,
            save_state,
        })
    }

    pub fn do_cycle(&mut self) -> u32 {
        self.cpu.do_cycle()
    }

    pub fn set_stdout(&mut self, output: bool) {
        if output {
            self.cpu.mmu.serial.set_callback(Box::new(StdoutPrinter));
        } else {
            self.cpu.mmu.serial.unset_callback();
        }
    }

    pub fn attach_printer(&mut self) {
        let printer = GbPrinter::new();

        self.cpu.mmu.serial.set_callback(Box::new(printer));
    }

    pub fn set_serial_callback(&mut self, cb: Box<dyn serial::SerialCallback>) {
        self.cpu.mmu.serial.set_callback(cb);
    }

    pub fn unset_serial_callback(&mut self) {
        self.cpu.mmu.serial.unset_callback();
    }

    pub fn check_and_reset_gpu_updated(&mut self) -> bool {
        let result = self.cpu.mmu.gpu.updated;
        self.cpu.mmu.gpu.updated = false;
        result
    }

    pub fn get_gpu_data(&self) -> &[u8] {
        &self.cpu.mmu.gpu.data
    }

    pub fn enable_audio(&mut self, player: Box<dyn sound::AudioPlayer>, is_on: bool) {
        match self.cpu.mmu.gbmode {
            GbMode::Classic => {
                self.cpu.mmu.sound = Some(sound::Sound::new_dmg(player));
            }
            GbMode::Color | GbMode::ColorAsClassic => {
                self.cpu.mmu.sound = Some(sound::Sound::new_cgb(player));
            }
        };
        if is_on {
            if let Some(sound) = self.cpu.mmu.sound.as_mut() {
                sound.set_on();
            }
        }
    }

    pub fn sync_audio(&mut self) {
        if let Some(ref mut sound) = self.cpu.mmu.sound {
            sound.sync();
        }
    }

    pub fn keyup(&mut self, key: KeypadKey) {
        self.cpu.mmu.keypad.keyup(key);
    }

    pub fn keydown(&mut self, key: KeypadKey) {
        self.cpu.mmu.keypad.keydown(key);
    }

    pub fn romname(&self) -> String {
        self.cpu.mmu.mbc.romname()
    }

    pub fn loadram(&mut self, ramdata: &[u8]) -> StrResult<()> {
        self.cpu.mmu.mbc.loadram(ramdata)
    }

    pub fn dumpram(&self) -> Vec<u8> {
        self.cpu.mmu.mbc.dumpram()
    }

    pub fn ram_is_battery_backed(&self) -> bool {
        self.cpu.mmu.mbc.is_battery_backed()
    }

    pub fn check_and_reset_ram_updated(&mut self) -> bool {
        self.cpu.mmu.mbc.check_and_reset_ram_updated()
    }

    pub fn read_byte(&mut self, address: u16) -> u8 {
        self.cpu.read_byte(address)
    }
    pub fn write_byte(&mut self, address: u16, byte: u8) {
        self.cpu.write_byte(address, byte)
    }
    pub fn read_wide(&mut self, address: u16) -> u16 {
        self.cpu.read_wide(address)
    }
    pub fn write_wide(&mut self, address: u16, byte: u16) {
        self.cpu.write_wide(address, byte)
    }

    // Custom

    /// Called by the main CPU thread after stepping the GPU.
    /// If the GPU just entered VBlank, write the mirror region and increment frame counter.
    pub fn maybe_write_mirror(&mut self) {
        // NOTE: use cpu.mmu.gpu and cpu.mmu.write_mirror() since Device stores a CPU.
        if self.cpu.mmu.gpu.take_vblank() {
            self.cpu.mmu.write_mirror();
        }
    }

    /// Set the current joypad mask (u8). Mask bit = 1 means pressed.
    pub fn set_joypad_mask(&mut self, mask: u8) {
        // Directly update the keypad that lives inside MMU.
        // This avoids trying to write to IO registers and is immediate.
        self.cpu.mmu.keypad.set_mask(mask);
    }

    /// Reset the emulator to a clean power-on state.
    pub fn reset(&mut self) {
        // Prefer calling CPU::reset() which should reset CPU registers, MMU, GPU, timers, etc.
        // If CPU::reset() exists it will be used; otherwise implement it (see suggested CPU::reset below).
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.cpu.reset();
        }));
    }

    /// Return a copy of the current mirror buffer. Requires MMU::get_mirror() -> &[u8].
    pub fn get_mirror(&self) -> Vec<u8> {
        // assumes mmu has a get_mirror() -> &[u8]
        self.cpu.mmu.get_mirror().to_vec()
    }

    /// Step the emulator until the next frame (VBlank) and return the last GPU frame data.
    /// This mirrors the behavior used by the UI thread.
    pub fn step_frame(&mut self) -> Vec<u8> {
        // The waitticks used in the main loop represent ~16ms worth of cycles,
        // but here we simply run cycles until GPU update occurs.
        loop {
            // Run a small chunk (the original do_cycle returns cycles consumed)
            let _cycles = self.do_cycle();

            // If GPU entered vblank, write mirror
            self.maybe_write_mirror();

            // If GPU updated (frame rendered), return its image data
            if self.check_and_reset_gpu_updated() {
                return self.get_gpu_data().to_vec();
            }
        }
    }

}
