use crate::gbmode::{GbMode, GbSpeed};
use crate::gpu::GPU;
use crate::keypad::Keypad;
use crate::mbc;
use crate::serial::{Serial, SerialCallback};
use crate::sound::Sound;
use crate::timer::Timer;
use crate::StrResult;
use serde::{Deserialize, Serialize};

const WRAM_SIZE: usize = 0x8000;
const ZRAM_SIZE: usize = 0x7F;

// Custom
// Pokemon G/S Memory
pub const MIRROR_FRAME_COUNTER: usize = 0x000;
pub const MIRROR_MAP_BANK: usize = 0x004;
pub const MIRROR_MAP_ID: usize = 0x005;
pub const MIRROR_PLAYER_X: usize = 0x006;
pub const MIRROR_PLAYER_Y: usize = 0x007;
pub const MIRROR_PARTY_COUNT: usize = 0x008;
pub const MIRROR_PARTY_START: usize = 0x009; // 6 × 11 bytes = 66 bytes
pub const MIRROR_IN_BATTLE: usize = 0x049;
pub const MIRROR_ENEMY_SPECIES: usize = 0x04A;
pub const MIRROR_ENEMY_LEVEL: usize = 0x04B;
pub const MIRROR_ENEMY_HP: usize = 0x04C; // 2 bytes
pub const MIRROR_MONEY: usize = 0x04E; // 3 bytes
pub const MIRROR_BADGES: usize = 0x051;
pub const MIRROR_SIZE: usize = 0x052; // 80 bytes

/* 
Mirror snapshot layout (little-endian) — for writing to fixed WRAM mirror region (e.g. 0xC000).
Goals:
 - All multi-byte fields use little-endian.
 - Single-byte fields unchanged.
 - Fixed offsets so snapshots are deterministic.
 - Default contains only "visible" player-observable fields (fairness).
 - Layout is expandable: reserved/padded space and a hidden/debug region at the end.

Layout (offsets are mirror-relative):
 0x000  4  Frame counter        -> u32 LE (increments each frame)
 0x004  1  Map bank             -> u8
 0x005  1  Map ID               -> u8
 0x006  1  Player X             -> u8
 0x007  1  Player Y             -> u8
 0x008  1  Party count          -> u8
 0x009 66  Party slots (6×11)   -> per slot: species, level, cur HP, max HP, status, moves (mostly u8)
 0x049  1  In battle            -> u8 (non-zero = in battle)
 0x04A  1  Enemy species        -> u8
 0x04B  1  Enemy level          -> u8
 0x04C  2  Enemy cur HP         -> u16 LE
 0x04E  2  Enemy max HP         -> u16 LE
 0x050  4  Money                -> u32 LE (convert from 3-byte BCD in MMU)
 0x054  1  Badges               -> u8 (bitfield)
 0x055  3  Padding / reserved   -> reserved for future expansion
 0x058 16  Hidden / debug       -> optional: RNG, IVs, internal flags, etc.

Total size: 0x068 (104 bytes)

Notes:
 - Keep the frame counter increment and mirror write atomic in MMU::write_mirror().
 - Expose MIRROR_SIZE and offsets as pub constants so Python/agents don't hardcode numbers.
 - For fairness, default bindings should expose only this "visible" mirror. Hidden/debug fields may be appended after MIRROR_SIZE or exposed via a flag.
*/
pub const MIRROR_SIZE: usize = 0x068

#[derive(PartialEq, Serialize, Deserialize)]
enum DMAType {
    NoDMA,
    GDMA,
    HDMA,
}

#[derive(Serialize, Deserialize)]
pub struct MMU {
    #[serde(with = "serde_arrays")]
    wram: [u8; WRAM_SIZE],
    #[serde(with = "serde_arrays")]
    zram: [u8; ZRAM_SIZE],
    hdma: [u8; 4],
    pub inte: u8,
    pub intf: u8,
    pub serial: Serial,
    pub timer: Timer,
    pub keypad: Keypad,
    pub gpu: GPU,
    #[serde(skip)]
    pub sound: Option<Sound>,
    hdma_status: DMAType,
    hdma_src: u16,
    hdma_dst: u16,
    hdma_len: u8,
    wrambank: usize,
    pub mbc: Box<dyn mbc::MBC + 'static>,
    pub gbmode: GbMode,
    gbspeed: GbSpeed,
    speed_switch_req: bool,
    undocumented_cgb_regs: [u8; 3], // 0xFF72, 0xFF73, 0xFF75

    // Custom
    wram_mirror: [u8; MIRROR_SIZE],
    frame_counter: u32,
}

fn fill_random(slice: &mut [u8], start: u32) {
    // Simple LCG to generate (non-cryptographic) random values
    // Each distinct invocation should use a different start value
    const A: u32 = 1103515245;
    const C: u32 = 12345;

    let mut x = start;
    for v in slice.iter_mut() {
        x = x.wrapping_mul(A).wrapping_add(C);
        *v = ((x >> 23) & 0xFF) as u8;
    }
}

impl MMU {
    pub fn new(
        cart: Box<dyn mbc::MBC + 'static>,
        serial_callback: Option<Box<dyn SerialCallback>>,
    ) -> StrResult<MMU> {
        let serial = match serial_callback {
            Some(cb) => Serial::new_with_callback(cb),
            None => Serial::new(),
        };
        let mut res = MMU {
            wram: [0; WRAM_SIZE],
            zram: [0; ZRAM_SIZE],
            hdma: [0; 4],
            wrambank: 1,
            inte: 0,
            intf: 0,
            serial: serial,
            timer: Timer::new(),
            keypad: Keypad::new(),
            gpu: GPU::new(),
            sound: None,
            mbc: cart,
            gbmode: GbMode::Classic,
            gbspeed: GbSpeed::Single,
            speed_switch_req: false,
            hdma_src: 0,
            hdma_dst: 0,
            hdma_status: DMAType::NoDMA,
            hdma_len: 0xFF,
            undocumented_cgb_regs: [0; 3],
        };
        fill_random(&mut res.wram, 42);
        if res.rb(0x0143) == 0xC0 {
            return Err("This game does not work in Classic mode");
        }
        res.set_initial();
        Ok(res)
    }

    pub fn new_cgb(
        cart: Box<dyn mbc::MBC + 'static>,
        serial_callback: Option<Box<dyn SerialCallback>>,
    ) -> StrResult<MMU> {
        let serial = match serial_callback {
            Some(cb) => Serial::new_with_callback(cb),
            None => Serial::new(),
        };
        let mut res = MMU {
            wram: [0; WRAM_SIZE],
            zram: [0; ZRAM_SIZE],
            wrambank: 1,
            hdma: [0; 4],
            inte: 0,
            intf: 0,
            serial: serial,
            timer: Timer::new(),
            keypad: Keypad::new(),
            gpu: GPU::new_cgb(),
            sound: None,
            mbc: cart,
            gbmode: GbMode::Color,
            gbspeed: GbSpeed::Single,
            speed_switch_req: false,
            hdma_src: 0,
            hdma_dst: 0,
            hdma_status: DMAType::NoDMA,
            hdma_len: 0xFF,
            undocumented_cgb_regs: [0; 3],
        };
        fill_random(&mut res.wram, 42);
        res.determine_mode();
        res.set_initial();
        Ok(res)
    }

    fn set_initial(&mut self) {
        self.wb(0xFF05, 0);
        self.wb(0xFF06, 0);
        self.wb(0xFF07, 0);
        self.wb(0xFF10, 0x80);
        self.wb(0xFF11, 0xBF);
        self.wb(0xFF12, 0xF3);
        self.wb(0xFF14, 0xBF);
        self.wb(0xFF16, 0x3F);
        self.wb(0xFF16, 0x3F);
        self.wb(0xFF17, 0);
        self.wb(0xFF19, 0xBF);
        self.wb(0xFF1A, 0x7F);
        self.wb(0xFF1B, 0xFF);
        self.wb(0xFF1C, 0x9F);
        self.wb(0xFF1E, 0xFF);
        self.wb(0xFF20, 0xFF);
        self.wb(0xFF21, 0);
        self.wb(0xFF22, 0);
        self.wb(0xFF23, 0xBF);
        self.wb(0xFF24, 0x77);
        self.wb(0xFF25, 0xF3);
        self.wb(0xFF26, 0xF1);
        self.wb(0xFF40, 0x91);
        self.wb(0xFF42, 0);
        self.wb(0xFF43, 0);
        self.wb(0xFF45, 0);
        self.wb(0xFF47, 0xFC);
        self.wb(0xFF48, 0xFF);
        self.wb(0xFF49, 0xFF);
        self.wb(0xFF4A, 0);
        self.wb(0xFF4B, 0);
    }

    fn determine_mode(&mut self) {
        let mode = match self.rb(0x0143) & 0x80 {
            0x80 => GbMode::Color,
            _ => GbMode::ColorAsClassic,
        };
        self.gbmode = mode;
        self.gpu.gbmode = mode;
    }

    pub fn do_cycle(&mut self, ticks: u32) -> u32 {
        let cpudivider = self.gbspeed as u32;
        let vramticks = self.perform_vramdma();
        let gputicks = ticks / cpudivider + vramticks;
        let cputicks = ticks + vramticks * cpudivider;

        self.timer.do_cycle(cputicks);
        self.intf |= self.timer.interrupt;
        self.timer.interrupt = 0;

        self.intf |= self.keypad.interrupt;
        self.keypad.interrupt = 0;

        self.gpu.do_cycle(gputicks);
        self.intf |= self.gpu.interrupt;
        self.gpu.interrupt = 0;

        let _ = self.sound.as_mut().map_or((), |s| s.do_cycle(gputicks));

        self.intf |= self.serial.interrupt;
        self.serial.interrupt = 0;

        return gputicks;
    }

    pub fn rb(&mut self, address: u16) -> u8 {
        match address {
            0x0000..=0x7FFF => self.mbc.readrom(address),
            0x8000..=0x9FFF => self.gpu.rb(address),
            0xA000..=0xBFFF => self.mbc.readram(address),
            0xC000..=0xCFFF | 0xE000..=0xEFFF => self.wram[address as usize & 0x0FFF],
            0xD000..=0xDFFF | 0xF000..=0xFDFF => {
                self.wram[(self.wrambank * 0x1000) | address as usize & 0x0FFF]
            }
            0xFE00..=0xFE9F => self.gpu.rb(address),
            0xFF00 => self.keypad.rb(),
            0xFF01..=0xFF02 => self.serial.rb(address),
            0xFF04..=0xFF07 => self.timer.rb(address),
            0xFF0F => self.intf | 0b11100000,
            0xFF10..=0xFF3F => self.sound.as_mut().map_or(0xFF, |s| s.rb(address)),
            0xFF4D | 0xFF4F | 0xFF51..=0xFF55 | 0xFF6C | 0xFF70 if self.gbmode != GbMode::Color => {
                0xFF
            }
            0xFF72..=0xFF73 | 0xFF75..=0xFF77 if self.gbmode == GbMode::Classic => 0xFF,
            0xFF4D => {
                0b01111110
                    | (if self.gbspeed == GbSpeed::Double {
                        0x80
                    } else {
                        0
                    })
                    | (if self.speed_switch_req { 1 } else { 0 })
            }
            0xFF40..=0xFF4F => self.gpu.rb(address),
            0xFF51..=0xFF55 => self.hdma_read(address),
            0xFF68..=0xFF6B => self.gpu.rb(address),
            0xFF70 => self.wrambank as u8,
            0xFF72..=0xFF73 => self.undocumented_cgb_regs[address as usize - 0xFF72],
            0xFF75 => self.undocumented_cgb_regs[2] | 0b10001111,
            0xFF76..=0xFF77 => 0x00, // CGB PCM registers. Not yet implemented.
            0xFF80..=0xFFFE => self.zram[address as usize & 0x007F],
            0xFFFF => self.inte,
            _ => 0xFF,
        }
    }

    pub fn rw(&mut self, address: u16) -> u16 {
        (self.rb(address) as u16) | ((self.rb(address + 1) as u16) << 8)
    }

    pub fn wb(&mut self, address: u16, value: u8) {
        match address {
            0x0000..=0x7FFF => self.mbc.writerom(address, value),
            0x8000..=0x9FFF => self.gpu.wb(address, value),
            0xA000..=0xBFFF => self.mbc.writeram(address, value),
            0xC000..=0xCFFF | 0xE000..=0xEFFF => self.wram[address as usize & 0x0FFF] = value,
            0xD000..=0xDFFF | 0xF000..=0xFDFF => {
                self.wram[(self.wrambank * 0x1000) | (address as usize & 0x0FFF)] = value
            }
            0xFE00..=0xFE9F => self.gpu.wb(address, value),
            0xFF00 => self.keypad.wb(value),
            0xFF01..=0xFF02 => self.serial.wb(address, value),
            0xFF04..=0xFF07 => self.timer.wb(address, value),
            0xFF10..=0xFF3F => self.sound.as_mut().map_or((), |s| s.wb(address, value)),
            0xFF46 => self.oamdma(value),
            0xFF4D | 0xFF4F | 0xFF51..=0xFF55 | 0xFF6C | 0xFF70 | 0xFF76..=0xFF77
                if self.gbmode != GbMode::Color => {}
            0xFF72..=0xFF73 | 0xFF75..=0xFF77 if self.gbmode == GbMode::Classic => {}
            0xFF4D => {
                if value & 0x1 == 0x1 {
                    self.speed_switch_req = true;
                }
            }
            0xFF40..=0xFF4F => self.gpu.wb(address, value),
            0xFF51..=0xFF55 => self.hdma_write(address, value),
            0xFF68..=0xFF6B => self.gpu.wb(address, value),
            0xFF0F => self.intf = value,
            0xFF70 => {
                self.wrambank = match value & 0x7 {
                    0 => 1,
                    n => n as usize,
                };
            }
            0xFF72..=0xFF73 => self.undocumented_cgb_regs[address as usize - 0xFF72] = value,
            0xFF75 => self.undocumented_cgb_regs[2] = value,
            0xFF80..=0xFFFE => self.zram[address as usize & 0x007F] = value,
            0xFFFF => self.inte = value,
            _ => {}
        };
    }

    pub fn ww(&mut self, address: u16, value: u16) {
        self.wb(address, (value & 0xFF) as u8);
        self.wb(address + 1, (value >> 8) as u8);
    }

    pub fn switch_speed(&mut self) {
        if self.speed_switch_req {
            if self.gbspeed == GbSpeed::Double {
                self.gbspeed = GbSpeed::Single;
            } else {
                self.gbspeed = GbSpeed::Double;
            }
        }
        self.speed_switch_req = false;
    }

    fn oamdma(&mut self, value: u8) {
        let base = (value as u16) << 8;
        for i in 0..0xA0 {
            let b = self.rb(base + i);
            self.wb(0xFE00 + i, b);
        }
    }

    fn hdma_read(&self, a: u16) -> u8 {
        match a {
            0xFF51..=0xFF54 => self.hdma[(a - 0xFF51) as usize],
            0xFF55 => {
                self.hdma_len
                    | if self.hdma_status == DMAType::NoDMA {
                        0x80
                    } else {
                        0
                    }
            }
            _ => panic!("The address {:04X} should not be handled by hdma_read", a),
        }
    }

    fn hdma_write(&mut self, a: u16, v: u8) {
        match a {
            0xFF51 => self.hdma[0] = v,
            0xFF52 => self.hdma[1] = v & 0xF0,
            0xFF53 => self.hdma[2] = v & 0x1F,
            0xFF54 => self.hdma[3] = v & 0xF0,
            0xFF55 => {
                if self.hdma_status == DMAType::HDMA {
                    if v & 0x80 == 0 {
                        self.hdma_status = DMAType::NoDMA;
                    };
                    return;
                }
                let src = ((self.hdma[0] as u16) << 8) | (self.hdma[1] as u16);
                let dst = ((self.hdma[2] as u16) << 8) | (self.hdma[3] as u16) | 0x8000;
                if !(src <= 0x7FF0 || (src >= 0xA000 && src <= 0xDFF0)) {
                    panic!("HDMA transfer with illegal start address {:04X}", src);
                }

                self.hdma_src = src;
                self.hdma_dst = dst;
                self.hdma_len = v & 0x7F;

                self.hdma_status = if v & 0x80 == 0x80 {
                    DMAType::HDMA
                } else {
                    DMAType::GDMA
                };
            }
            _ => panic!("The address {:04X} should not be handled by hdma_write", a),
        };
    }

    fn perform_vramdma(&mut self) -> u32 {
        match self.hdma_status {
            DMAType::NoDMA => 0,
            DMAType::GDMA => self.perform_gdma(),
            DMAType::HDMA => self.perform_hdma(),
        }
    }

    fn perform_hdma(&mut self) -> u32 {
        if self.gpu.may_hdma() == false {
            return 0;
        }

        self.perform_vramdma_row();
        if self.hdma_len == 0x7F {
            self.hdma_status = DMAType::NoDMA;
        }

        return 8;
    }

    fn perform_gdma(&mut self) -> u32 {
        let len = self.hdma_len as u32 + 1;
        for _i in 0..len {
            self.perform_vramdma_row();
        }

        self.hdma_status = DMAType::NoDMA;
        return len * 8;
    }

    fn perform_vramdma_row(&mut self) {
        let mmu_src = self.hdma_src;
        for j in 0..0x10 {
            let b: u8 = self.rb(mmu_src + j);
            self.gpu.wb(self.hdma_dst + j, b);
        }
        self.hdma_src += 0x10;
        self.hdma_dst += 0x10;

        if self.hdma_len == 0 {
            self.hdma_len = 0x7F;
        } else {
            self.hdma_len -= 1;
        }
    }

    // Custom
    pub fn write_mirror(&mut self) {
        // --- frame counter ---
        self.frame_counter = self.frame_counter.wrapping_add(1);
        self.mirror[0x000..0x004].copy_from_slice(&self.frame_counter.to_le_bytes());

        // --- map & player ---
        self.mirror[0x004] = self.wram[0xDA00]; // map bank
        self.mirror[0x005] = self.wram[0xDA01]; // map ID
        self.mirror[0x006] = self.wram[0xD20D]; // X
        self.mirror[0x007] = self.wram[0xD20E]; // Y

        // --- party ---
        self.mirror[0x008] = self.wram[0xDA22]; // party count
        for i in 0..6 {
            let src = 0xDA2A + i*11;
            let dst = 0x009 + i*11;
            self.mirror[dst..dst+11].copy_from_slice(&self.wram[src..src+11]);
        }

        // --- battle state ---
        self.mirror[0x049] = self.wram[0xD116];      // in battle
        self.mirror[0x04A] = self.wram[0xD0ED];      // enemy species
        self.mirror[0x04B] = self.wram[0xD0FC];      // enemy level

        let enemy_cur_hp = u16::from_be_bytes([self.wram[0xD0FF], self.wram[0xD100]]);
        self.mirror[0x04C..0x04E].copy_from_slice(&enemy_cur_hp.to_le_bytes());

        let enemy_max_hp = u16::from_be_bytes([self.wram[0xD101], self.wram[0xD102]]);
        self.mirror[0x04E..0x050].copy_from_slice(&enemy_max_hp.to_le_bytes());

        // --- money (3-byte BCD -> u32 LE) ---
        let bcd = &self.wram[0xD573..0xD576];
        let money = (bcd[0] as u32)*10000 + (bcd[1] as u32)*100 + (bcd[2] as u32);
        self.mirror[0x050..0x054].copy_from_slice(&money.to_le_bytes());

        // --- badges ---
        self.mirror[0x054] = self.wram[0xD57C];

        // --- padding / reserved ---
        self.mirror[0x055..0x058].fill(0);

        // --- optional hidden/debug ---
        // Example: copy RNG state for debugging
        self.mirror[0x058..0x05A].copy_from_slice(&self.wram[0xFFD3..0xFFD5]);
        // remaining bytes (0x05A..0x068) can be used later for IVs, encounter cooldowns, etc.
    }

    pub fn get_mirror(&self) -> &[u8] {
        &self.mirror[..MIRROR_SIZE]
    }

    pub fn reset(&mut self) {
        // Clear WRAM/HRAM/VRAM/OAM to expected power-on values
        for b in self.wram.iter_mut() { *b = 0; }
        for b in self.vram.iter_mut() { *b = 0; }
        for b in self.oam.iter_mut()  { *b = 0; }
        // Reset IO registers to their default values (implement individually)
        self.io_reset();
        // Reset keypad state
        self.keypad = crate::keypad::Keypad::new();
        // Reset GPU and sound if needed (but GPU::new() will be called by CPU reset above)
    }

    fn io_reset(&mut self) {
        // Write default power-on values for IO addresses if you want
        // e.g. self.write_byte(0xFF00, 0xFF); // JOYP
        // implement the small set of default IO registers your emulator requires
    }
}
