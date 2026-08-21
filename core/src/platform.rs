pub fn sustain_background_performance() {
    let power = opt_out_of_power_throttling();
    tracing::debug!(power, "power-throttling opt-out applied");
}

#[cfg(not(windows))]
fn opt_out_of_power_throttling() -> bool {
    true
}

#[cfg(windows)]
fn opt_out_of_power_throttling() -> bool {
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
        ProcessPowerThrottling, SetProcessInformation,
    };

    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: 0,
    };
    let ok = unsafe {
        SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            std::ptr::from_ref(&state).cast(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
    };
    if ok == 0 {
        let error = std::io::Error::last_os_error();
        tracing::debug!(%error, "power throttling opt-out not applied");
    }
    ok != 0
}
