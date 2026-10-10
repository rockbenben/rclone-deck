// release 构建必须是 windows 子系统，否则面板自己就带一个控制台窗口。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    rclone_deck_lib::run()
}
