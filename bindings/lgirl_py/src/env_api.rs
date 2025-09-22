use pyo3::exceptions;
use pyo3::prelude::*;
use pyo3::types::PyBytes;

// Import your crate by its new name
use rgirl;
use rgirl::device::Device;

/// A tiny helper to expose mirror size constant to Python (change if you have a MIRROR_SIZE export)
#[pyfunction]
fn mirror_size() -> usize {
    // If your mmu exports MIRROR_SIZE: return it. Otherwise return fallback 0x68 (104)
    rgirl::mmu::MIRROR_SIZE
}

#[pymodule]
fn rgirl_env(py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<Env>()?;
    m.add_function(wrap_pyfunction!(mirror_size, m)?)?;
    Ok(())
}

#[pyclass]
pub struct Env {
    dev: Device,
}

#[pymethods]
impl Env {
    /// __new__(rom_path: str, *, skip_checksum: bool=False, classic_mode: bool=False)
    #[new]
    fn new(rom_path: String, skip_checksum: Option<bool>, classic_mode: Option<bool>) -> PyResult<Self> {
        let skip = skip_checksum.unwrap_or(false);
        let classic = classic_mode.unwrap_or(false);

        let dev_res = if classic {
            Device::new_cgb(&rom_path, skip, None)
        } else {
            Device::new(&rom_path, skip, None)
        };

        match dev_res {
            Ok(dev) => Ok(Env { dev }),
            Err(e) => Err(PyErr::new::<exceptions::PyRuntimeError, _>(format!("Failed to create Device: {}", e))),
        }
    }

    fn reset(&mut self) -> PyResult<()> {
        self.dev.reset();
        Ok(())
    }

    /// set the single-byte action mask (u8). The semantics of the mask are up to Python-side.
    fn set_action(&mut self, mask: u8) -> PyResult<()> {
        self.dev.set_joypad_mask(mask);
        Ok(())
    }

    /// step(action: u8) -> (mirror_bytes, reward, done)
    fn step<'p>(&mut self, py: Python<'p>, action: u8) -> PyResult<(&'p PyBytes, f32, bool)> {
        // Apply action
        self.dev.set_joypad_mask(action);

        // Step until next frame and ensure mirror updated
        let _frame = self.dev.step_frame(); // we don't need the image here

        // Read mirror
        let mirror_vec = self.dev.get_mirror();
        let pyb = PyBytes::new(py, &mirror_vec);

        // Placeholder reward / done — compute in Python from mirror for now
        Ok((pyb, 0.0_f32, false))
    }

    /// get_mirror() -> bytes
    fn get_mirror<'p>(&self, py: Python<'p>) -> PyResult<&'p PyBytes> {
        let mirror_vec = self.dev.get_mirror();
        Ok(PyBytes::new(py, &mirror_vec))
    }
}
