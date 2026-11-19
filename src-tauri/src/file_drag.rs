//! Native virtual-file drag-out. File contents are fetched only when a drop
//! target requests them; only COPY is offered, never MOVE of the remote file.
use crate::{sftp_transfer::TransferProgress, AppState, TransferResult};
use tauri::{ipc::Channel, AppHandle, Manager};

#[tauri::command]
pub async fn sftp_drag_download(
    session_id: String,
    transfer_id: String,
    path: String,
    progress: Channel<TransferProgress>,
    app: AppHandle,
) -> Result<Option<TransferResult>, String> {
    #[cfg(not(windows))]
    {
        let _ = (session_id, transfer_id, path, progress, app);
        Err("当前平台暂不支持拖出下载，请使用下载按钮选择保存位置。".into())
    }
    #[cfg(windows)]
    {
        use orbitshell_ssh::file_drag::{self as native, DragOptions};
        let lease = native::DragLease::acquire()?;
        let live = app.state::<AppState>().session(&session_id)?;
        let name = native::filename(&path)?;
        let result_name = name.clone();
        let task = app.state::<AppState>().transfers.manual(
            &app,
            transfer_id,
            &live,
            &session_id,
            crate::file_transfers::Direction::Download,
            name.clone(),
            String::new(),
            path.clone(),
        )?;
        task.message(&app, "等待选择拖放目标");
        let (send, receive) = tokio::sync::oneshot::channel();
        let drag_app = app.clone();
        let drag_task = task.clone();
        // OLE must track the source window's mouse-message queue. Run its drag
        // loop on that UI thread; GetData pumps COM messages while SFTP runs
        // asynchronously so paints, transfer progress and cancellation continue.
        let scheduled = app.run_on_main_thread(move || {
            let _lease = lease;
            let scale = drag_app
                .get_webview_window("main")
                .and_then(|window| window.scale_factor().ok())
                .unwrap_or(1.0);
            let app_for_progress = drag_app.clone();
            let result = native::run(
                live,
                path,
                name,
                DragOptions {
                    runtime: tauri::async_runtime::handle().inner().clone(),
                    scale,
                    cancel: tokio_util::sync::CancellationToken::new(),
                    progress: Box::new(move |event| {
                        drag_task.progress(&app_for_progress, event.clone());
                        let _ = progress.send(event);
                    }),
                    should_stop: Box::new(move || {
                        drag_app
                            .state::<AppState>()
                            .desktop
                            .exiting
                            .load(std::sync::atomic::Ordering::Acquire)
                    }),
                },
            );
            let _ = send.send(result);
        });
        let result = match scheduled {
            Ok(_) => receive
                .await
                .map_err(|_| "系统拖放未完成，请重新拖拽".to_string())
                .and_then(|result| result),
            Err(error) => Err(format!("无法在窗口线程启动系统拖放：{error}")),
        };
        app.state::<AppState>()
            .transfers
            .finish_drag(&app, &task, &result);
        result.map(|bytes| {
            bytes.map(|bytes| TransferResult {
                name: result_name,
                bytes,
            })
        })
    }
}
