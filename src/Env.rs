use pyo3::prelude::*;
use pyo3::types::PyBytes;

#[pyclass]
pub struct Env {
    emulator: rgirl::Emulator,  // your main emulator struct
}

#[pymethods]
impl Env {
    #[new]
    fn new(rom_path: String) -> PyResult<Self> {
        let emulator = rgirl::Emulator::new(&rom_path);
        Ok(Env { emulator })
    }

    fn reset(&mut self) {
        self.emulator.reset();
    }

    fn step(&mut self, action: u8) -> PyResult<(PyObject, f32, bool)> {
        let reward = self.emulator.step(action);
        let done = self.emulator.is_done();
        Python::with_gil(|py| {
            let mirror_bytes = PyBytes::new(py, self.emulator.mmu.get_mirror());
            Ok((mirror_bytes.into(), reward, done))
        })
    }

    fn get_mirror(&self) -> PyResult<PyObject> {
        Python::with_gil(|py| {
            let mirror_bytes = PyBytes::new(py, self.emulator.mmu.get_mirror());
            Ok(mirror_bytes.into())
        })
    }
}
