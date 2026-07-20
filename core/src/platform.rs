pub fn sustain_background_performance() {
    let power = opt_out_of_power_throttling();
    let gpu = raise_gpu_scheduling_priority();
    tracing::debug!(power, gpu, "background-performance opt-outs applied");
}

#[cfg(not(windows))]
fn opt_out_of_power_throttling() -> bool {
    true
}

#[cfg(not(windows))]
fn raise_gpu_scheduling_priority() -> bool {
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

#[cfg(windows)]
fn raise_gpu_scheduling_priority() -> bool {
    use windows_sys::Wdk::Graphics::Direct3D::{
        D3DKMT_SCHEDULINGPRIORITYCLASS_ABOVE_NORMAL, D3DKMT_SCHEDULINGPRIORITYCLASS_HIGH,
        D3DKMTSetProcessSchedulingPriorityClass,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    for class in [
        D3DKMT_SCHEDULINGPRIORITYCLASS_HIGH,
        D3DKMT_SCHEDULINGPRIORITYCLASS_ABOVE_NORMAL,
    ] {
        let status = unsafe { D3DKMTSetProcessSchedulingPriorityClass(GetCurrentProcess(), class) };
        if status == 0 {
            return true;
        }
        tracing::debug!(class, status, "GPU scheduling priority class rejected");
    }
    false
}

#[cfg(test)]
mod tests {
    #[test]
    fn gpu_scheduling_priority_can_be_raised() {
        assert!(super::raise_gpu_scheduling_priority());
    }
}
