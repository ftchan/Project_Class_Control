//! Windows service integration (Windows-only).
//!
//! The agent runs as a standard Windows service. "Keep the client running" is
//! implemented with Service Control Manager recovery actions. While the
//! service runs, it also restores the per-user tray UI after an unexpected
//! exit. Administrators can stop or remove the service with `sc.exe`,
//! `services.msc`, or `labctl-student uninstall`.

use anyhow::Result;
use std::ffi::OsString;
use std::time::Duration;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::{
        Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock},
        RemoteDesktop::{WTSGetActiveConsoleSessionId, WTSQueryUserToken},
        Threading::{
            CreateProcessAsUserW, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, STARTUPINFOW,
        },
    },
};
use windows_service::{
    service::{
        ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept,
        ServiceErrorControl, ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod,
        ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
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

pub fn is_running() -> bool {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT);
    manager.ok().and_then(|m| m.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS).ok())
        .and_then(|s| s.query_status().ok())
        .is_some_and(|s| s.current_state == ServiceState::Running)
}

fn service_main(_args: Vec<OsString>) {
    if let Err(e) = run_service_inner() {
        tracing::error!(error = %e, "service terminated with error");
    }
}

fn run_service_inner() -> Result<()> {
    crate::init_tracing(Some(labctl_common::paths::log_dir().join("agent.log")));

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
    // The tray UI lives in the signed-in user's session. An HKLM Run entry
    // starts it at logon; this service also restores it after an unexpected
    // termination (for example, End task in Task Manager).
    let ui_recovery = rt.spawn(recover_user_ui());
    let result = rt.block_on(crate::agent::run(crate::agent::ShutdownSignal::Channel(
        shutdown_rx,
    )));
    ui_recovery.abort();

    set_state(ServiceState::Stopped, ServiceControlAccept::empty())?;
    result
}

/// Check the interactive UI while the service is running. Leave enough time
/// for logon autostart and for a freshly launched UI to bind its control port.
async fn recover_user_ui() {
    tokio::time::sleep(Duration::from_secs(10)).await;
    loop {
        if std::net::TcpStream::connect_timeout(
            &(
                std::net::Ipv4Addr::LOCALHOST,
                labctl_common::UI_CONTROL_PORT,
            )
                .into(),
            Duration::from_millis(300),
        )
        .is_err()
        {
            match launch_ui_in_console_session(false) {
                Ok(true) => {
                    tracing::info!("restored student UI in the console session");
                    tokio::time::sleep(Duration::from_secs(10)).await;
                }
                Ok(false) => {} // No interactive user is signed in.
                Err(e) => tracing::warn!(error = %e, "could not restore student UI"),
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

/// Launch only our installed executable, using the active console user's
/// token. A service process itself cannot display a window in session 0.
pub(crate) fn launch_ui_in_console_session(show: bool) -> Result<bool> {
    use std::os::windows::ffi::OsStrExt;

    let session = unsafe { WTSGetActiveConsoleSessionId() };
    if session == u32::MAX {
        return Ok(false);
    }

    let mut token = HANDLE::default();
    if unsafe { WTSQueryUserToken(session, &mut token) }.is_err() {
        return Ok(false); // Console is at the sign-in screen.
    }

    let result = (|| -> Result<()> {
        let exe = std::env::current_exe()?;
        let application: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut command: Vec<u16> = vec![b'"' as u16];
        command.extend(exe.as_os_str().encode_wide());
        command.extend(if show { "\" ui --show\0" } else { "\" ui\0" }.encode_utf16());
        let mut desktop: Vec<u16> = "winsta0\\default\0".encode_utf16().collect();
        let mut startup = STARTUPINFOW::default();
        startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        startup.lpDesktop = PWSTR(desktop.as_mut_ptr());
        let mut process = PROCESS_INFORMATION::default();
        let mut environment = std::ptr::null_mut();

        unsafe {
            CreateEnvironmentBlock(&mut environment, token, false)?;
            let created = CreateProcessAsUserW(
                token,
                PCWSTR(application.as_ptr()),
                PWSTR(command.as_mut_ptr()),
                None,
                None,
                false,
                CREATE_UNICODE_ENVIRONMENT,
                Some(environment),
                PCWSTR::null(),
                &startup,
                &mut process,
            );
            let _ = DestroyEnvironmentBlock(environment);
            created?;
            let _ = CloseHandle(process.hThread);
            let _ = CloseHandle(process.hProcess);
        }
        Ok(())
    })();
    unsafe {
        let _ = CloseHandle(token);
    }
    result.map(|()| true)
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

    let service =
        manager.create_service(&info, ServiceAccess::CHANGE_CONFIG | ServiceAccess::START)?;
    service.set_description("Enforces administrator lab policy on this managed client.")?;

    // Standard recovery: restart after a short delay on unexpected failure.
    let actions = ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(86_400)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![
            ServiceAction {
                action_type: ServiceActionType::Restart,
                delay: Duration::from_secs(5),
            },
            ServiceAction {
                action_type: ServiceActionType::Restart,
                delay: Duration::from_secs(10),
            },
            ServiceAction {
                action_type: ServiceActionType::Restart,
                delay: Duration::from_secs(30),
            },
        ]),
    };
    service.update_failure_actions(actions)?;

    service.start::<&str>(&[])?;
    println!("Installed and started service '{SERVICE_NAME}'.");
    Ok(())
}

pub fn uninstall() -> Result<()> {
    const ERROR_SERVICE_DOES_NOT_EXIST: i32 = 1060;

    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let service = match manager.open_service(
        SERVICE_NAME,
        ServiceAccess::STOP | ServiceAccess::DELETE | ServiceAccess::QUERY_STATUS,
    ) {
        Ok(s) => s,
        Err(e) => {
            // Not installed is not an error — makes a pre-clean before install
            // idempotent (and avoids writing to stderr / non-zero exit).
            if matches!(&e, windows_service::Error::Winapi(io)
                if io.raw_os_error() == Some(ERROR_SERVICE_DOES_NOT_EXIST))
            {
                println!("Service '{SERVICE_NAME}' is not installed; nothing to remove.");
                return Ok(());
            }
            return Err(e.into());
        }
    };

    let status = service.query_status()?;
    if status.current_state != ServiceState::Stopped {
        let _ = service.stop();
        std::thread::sleep(Duration::from_secs(2));
    }
    service.delete()?;
    println!("Removed service '{SERVICE_NAME}'.");
    Ok(())
}
