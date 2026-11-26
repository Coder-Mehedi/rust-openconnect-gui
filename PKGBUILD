# Local-development PKGBUILD: builds from the checkout it lives in.
# Install with:  makepkg -si
pkgname=rust-openconnect-gui
pkgver=0.1.0
pkgrel=1
pkgdesc="Minimal Rust/egui GUI for openconnect VPN — saved passwords, unattended connects"
arch=('x86_64')
license=('MIT')
depends=('openconnect' 'sudo' 'procps-ng')
makedepends=('cargo')
options=('!debug')

build() {
    cd "$startdir"
    cargo build --release --locked
}

package() {
    cd "$startdir"
    install -Dm755 target/release/rust-openconnect-gui "$pkgdir/usr/bin/rust-openconnect-gui"
    install -Dm644 assets/rust-openconnect-gui.desktop "$pkgdir/usr/share/applications/rust-openconnect-gui.desktop"
    install -Dm644 assets/rust-openconnect-gui.svg "$pkgdir/usr/share/icons/hicolor/scalable/apps/rust-openconnect-gui.svg"
}
