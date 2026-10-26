//! Browser entry point: hydrate the prerendered page.

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(bcc_web::App);
}
