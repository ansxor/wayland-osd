# Maintainer: Your Name <your.email@example.com>

pkgname=wayland-osd
pkgver=0.1.0
pkgrel=1
pkgdesc="On-screen display for Wayland"
arch=('x86_64')
url="https://example.com"
license=('MIT')
depends=()
makedepends=('cargo')

build() {
    cd "$srcdir/../wayland-osd-server"
    cargo build --release

    cd "$srcdir/../wayland-osd-client"
    cargo build --release
}

package() {
    cd "$srcdir/../wayland-osd-server"
    install -Dm755 "target/release/wayland-osd-server" "$pkgdir/usr/bin/wayland-osd-server"

    cd "$srcdir/../wayland-osd-client"
    install -Dm755 "target/release/wayland-osd-client" "$pkgdir/usr/bin/wayland-osd-client"
}