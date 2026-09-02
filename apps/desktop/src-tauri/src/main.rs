// The console is the window; a second console window behind it on Windows is
// not part of the product.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tam_desktop::run();
}
