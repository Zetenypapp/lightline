// Puts the app icon into the .exe (Explorer, Start menu, taskbar pins).
fn main() {
    embed_resource::compile("lightline.rc", embed_resource::NONE).manifest_optional().unwrap();
}
