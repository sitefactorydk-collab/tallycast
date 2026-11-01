//! Which DXGI output (monitor) is which, for FFmpeg's Desktop Duplication grabber.
//!
//! `ddagrab=output_idx=N` counts the outputs of the *default* graphics adapter (the
//! first one DXGI enumerates). Its log only reports sizes, so the index of a monitor is
//! looked up here by its desktop position. Plain COM calls through the vtable keep this
//! free of extra crates.

/// The `output_idx` of the monitor whose top-left corner is at (`x`, `y`) in physical
/// desktop pixels, if that monitor is driven by the default adapter.
pub fn output_index(x: i32, y: i32, width: u32, height: u32) -> Option<u32> {
    imp::outputs().into_iter().find_map(|o| {
        let (w, h) = ((o.right - o.left) as u32, (o.bottom - o.top) as u32);
        // Rotated displays report their size swapped; the position is what matters.
        (o.left == x && o.top == y && ((w, h) == (width, height) || (h, w) == (width, height)))
            .then_some(o.index)
    })
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct Output {
    pub index: u32,
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[cfg(windows)]
mod imp {
    use super::Output;
    use std::ffi::c_void;
    use std::ptr;

    #[repr(C)]
    struct Guid {
        d1: u32,
        d2: u16,
        d3: u16,
        d4: [u8; 8],
    }

    /// IID_IDXGIFactory1 {770aae78-f26f-4dba-a829-253c83d1b387}
    const IID_IDXGI_FACTORY1: Guid = Guid {
        d1: 0x770a_ae78,
        d2: 0xf26f,
        d3: 0x4dba,
        d4: [0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87],
    };

    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    /// DXGI_OUTPUT_DESC
    #[allow(dead_code)]
    #[repr(C)]
    struct OutputDesc {
        device_name: [u16; 32],
        desktop: Rect,
        attached: i32,
        rotation: u32,
        monitor: *mut c_void,
    }

    #[link(name = "dxgi")]
    extern "system" {
        fn CreateDXGIFactory1(riid: *const Guid, factory: *mut *mut c_void) -> i32;
    }

    // Vtable slots: IUnknown 0–2, IDXGIObject 3–6, then the interface's own methods.
    const RELEASE: usize = 2;
    const FACTORY_ENUM_ADAPTERS: usize = 7;
    const ADAPTER_ENUM_OUTPUTS: usize = 7;
    const OUTPUT_GET_DESC: usize = 7;

    type EnumFn = unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32;
    type DescFn = unsafe extern "system" fn(*mut c_void, *mut OutputDesc) -> i32;
    type ReleaseFn = unsafe extern "system" fn(*mut c_void) -> u32;

    unsafe fn slot(obj: *mut c_void, index: usize) -> *const c_void {
        let vtbl = *(obj as *const *const *const c_void);
        *vtbl.add(index)
    }

    unsafe fn release(obj: *mut c_void) {
        if !obj.is_null() {
            let f: ReleaseFn = std::mem::transmute(slot(obj, RELEASE));
            f(obj);
        }
    }

    pub fn outputs() -> Vec<Output> {
        let mut found = Vec::new();
        unsafe {
            let mut factory: *mut c_void = ptr::null_mut();
            if CreateDXGIFactory1(&IID_IDXGI_FACTORY1, &mut factory) < 0 || factory.is_null() {
                return found;
            }
            let enum_adapters: EnumFn = std::mem::transmute(slot(factory, FACTORY_ENUM_ADAPTERS));
            let mut adapter: *mut c_void = ptr::null_mut();
            // Adapter 0 is the one D3D11 (and so FFmpeg's ddagrab) uses by default.
            if enum_adapters(factory, 0, &mut adapter) >= 0 && !adapter.is_null() {
                let enum_outputs: EnumFn = std::mem::transmute(slot(adapter, ADAPTER_ENUM_OUTPUTS));
                for i in 0..16u32 {
                    let mut output: *mut c_void = ptr::null_mut();
                    if enum_outputs(adapter, i, &mut output) < 0 || output.is_null() {
                        break; // DXGI_ERROR_NOT_FOUND: no more outputs
                    }
                    let get_desc: DescFn = std::mem::transmute(slot(output, OUTPUT_GET_DESC));
                    let mut desc: OutputDesc = std::mem::zeroed();
                    if get_desc(output, &mut desc) >= 0 {
                        let r = &desc.desktop;
                        found.push(Output {
                            index: i,
                            left: r.left,
                            top: r.top,
                            right: r.right,
                            bottom: r.bottom,
                        });
                    }
                    release(output);
                }
                release(adapter);
            }
            release(factory);
        }
        found
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn outputs() -> Vec<super::Output> {
        Vec::new()
    }
}
