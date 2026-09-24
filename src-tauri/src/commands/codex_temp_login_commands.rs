// Codex 官方客户端临时登录命令。
//
// 本片段由 `commands/codex.rs` 通过 include! 纳入 `commands::codex` 模块，
// 对外调用仍使用 commands::codex 下的同名命令，不改变既有命令路径。

/// 打开官方客户端完成一次登录：使用一次性的空白临时 profile。
///
/// 读取到登录信息后会立即关闭官方客户端并清理临时 profile（含 macOS 钥匙串条目），
/// 成功、失败、取消都会走同一套清理逻辑；进度通过 codex:temp-login-progress 事件回传。
///
/// `intercept_auth_url` 为 true 时，官方客户端的"打开浏览器"会被接管：授权地址直接
/// 显示在弹框里（地址由官方生成，不做改写），不再弹浏览器；关闭时完全走官方原生流程。
#[tauri::command]
pub fn start_codex_temp_login(
    _app: AppHandle,
    _intercept_auth_url: bool,
) -> Result<crate::modules::codex_temp_login::CodexTempLoginSession, String> {
    Err("隔离版请使用添加账号窗口中的 OAuth 代理授权；官方客户端临时登录无法保证浏览器出口".into())
}

/// 取消官方客户端登录（仍会关闭客户端并清理临时 profile）。
#[tauri::command]
pub fn cancel_codex_temp_login(session_id: String) -> Result<(), String> {
    crate::modules::codex_temp_login::cancel(session_id.as_str())
}

/// 用默认浏览器打开本次会话截获到的官方授权地址。
///
/// 地址由官方客户端生成并原样保存，这里只校验官方域名后交给系统浏览器。
#[tauri::command]
pub fn open_codex_temp_login_auth_url(_app: AppHandle, _url: String) -> Result<(), String> {
    Err("隔离版已停用默认浏览器授权入口，请使用代理 OAuth 窗口".into())
}

/// 手动触发一次残留清理（正常由启动巡检与定期巡检自动完成）。
#[tauri::command]
pub fn cleanup_codex_temp_login_artifacts() -> Result<
    crate::modules::codex_temp_login::CodexTempLoginCleanupReport,
    String,
> {
    Ok(crate::modules::codex_temp_login::cleanup_stale_sessions())
}
