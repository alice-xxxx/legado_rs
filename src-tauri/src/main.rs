//! Tauri 程序入口；平台初始化与命令注册由 legado_lib 承担。
// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    legado_lib::run()
}
