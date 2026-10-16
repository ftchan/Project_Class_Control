//! Windows service integration (Windows-only).
//!
//! The agent runs as a standard Windows service. "Keep the client running" is
//! implemented the ordinary, supported way: the Service Control Manager's
//! recovery actions restart the service if it exits unexpectedly. This is the
//! same mechanism antivirus, backup, and management agents use. It is **not**
//! an attempt to hide the process or defeat Task Manager — an administrator
//! can always stop or remove the service with `sc.exe` / `services.msc` or
//! `labctl-student uninstall`.

use anyhow::Result;
use std::ffi::OsString;
use std::time::Duration;
use windows_service::{
    service::{
        ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
        ServiceFailureActions, ServiceFailureResetPeriod, ServiceInfo, ServiceStartType,
        ServiceState, ServiceStatus, ServiceType, ServiceAction, ServiceActionType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
    service_manager::{ServiceManager, ServiceManagerAccess},
};

const SERVICE_NAME: &str = "LabCtlStudent";
const DISPLAY_NAME: &str = "Lab Classroom Managed Client";

windows_service::define_windows_service!(ffi_service_main, service_main);

pub fn run_as_service() -> Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)?;
    Ok(())
}

fn service_main(_args: Vec<OsString>) {
    if let Err(e) = run_service_inner() {
        tracing::error!(error = %e, "service terminated with error");
    }
}

fn run_service_inner() -> Result<()> {
    crate::init_tracing();

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let mut shutdown_tx = Some(shutdown_tx);

    let event_handler = move |control: ServiceControl| -> ServiceControlHandlerResult {
        match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                if let Some(tx) = shutdown_tx.take() {
                    let _ = tx.send(());
                }
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;

    let set_state = |state: ServiceState, accept: ServiceControlAccept| {
        status_handle.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accept,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        })
    };

    set_state(ServiceState::StartPending, ServiceControlAccept::empty())?;
    set_state(
        ServiceState::Running,
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
    )?;

    let rt = tokio::runtime::Runtime::new()?;
    let result = rt.block_on(crate::agent::run(crate::agent::ShutdownSignal::Channel(shutdown_rx)));

    set_state(ServiceState::Stopped, ServiceControlAccept::empty())?;
    result
}

pub fn install() -> Result<()> {
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )?;

    let exe = std::env::current_exe()?;
    let info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(DISPLAY_NAME),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe,
        launch_arguments: vec![OsString::from("service")],
        dependencies: vec![],
        account_name: None, // LocalSystem
        account_password: None,
    };

    let service = manager.create_service(
        &info,
        ServiceAccess::CHANGE_CONFIG | ServiceAccess::START,
    )?;
    service.set_description("Enforces administrator lab policy on this managed client.")?;

    // Standard recovery: restart after a short delay on unexpected failure.
    let actions = ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(86_400)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![
            ServiceAction { action_type: ServiceActionType::Restart, delay: Duration::from_secs(5) },
            ServiceAction { action_type: ServiceActionType::Restart, delay: Duration::from_secs(10) },
            ServiceAction { action_type: ServiceActionType::Restart, delay: Duration::from_secs(30) },
        ]),
    };
    service.update_failure_actions(actions)?;

    service.start::<&str>(&[])?;
    println!("Installed and started service '{SERVICE_NAME}'.");
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let service = manager.open_service(
        SERVICE_NAME,
        ServiceAccess::STOP | ServiceAccess::DELETE | ServiceAccess::QUERY_STATUS,
    )?;

    let status = service.query_status()?;
    if status.current_state != ServiceState::Stopped {
        let _ = service.stop();
        std::thread::sleep(Duration::from_secs(2));
    }
    service.delete()?;
    println!("Removed service '{SERVICE_NAME}'.");
    Ok(())
}
