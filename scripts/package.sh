#!/usr/bin/env bash
# Gera pacotes em dist/. Local e CI usam este mesmo script.
#   scripts/package.sh deb | rpm | arch | windows | msi | macos | linux (= deb rpm) | version X.Y.Z
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
WIN=x86_64-pc-windows-msvc
mkdir -p dist

need() { command -v "$1" >/dev/null || { echo "faltando '$1': $2" >&2; exit 1; }; }
release() { cargo build --release --locked; }

deb() {
  need cargo-deb "cargo install cargo-deb"
  release
  cargo deb --no-build --output "dist/pdrop_${VERSION}_amd64.deb"
}

rpm() {
  need cargo-generate-rpm "cargo install cargo-generate-rpm"
  release
  cargo generate-rpm --output "dist/pdrop-${VERSION}-1.x86_64.rpm"
}

# Arch: roda makepkg num container (não precisa ser Arch). Usa o código local, não o GitHub.
arch() {
  local ctr
  ctr=$(command -v podman || command -v docker) || { echo "faltando podman ou docker" >&2; exit 1; }
  local work
  work=$(mktemp -d)
  tar czf "$work/pdrop-$VERSION.tar.gz" --exclude=./target --exclude=./dist --exclude=./.git \
    --transform "s,^\.,pdrop-$VERSION," .
  sed 's|^source=.*|source=("pdrop-$pkgver.tar.gz")|' packaging/arch/PKGBUILD > "$work/PKGBUILD"
  # o build roda numa cópia dentro do container: o host nunca perde acesso a $work
  "$ctr" run --rm --network=host -v "$work:/work:ro,Z" -v "$PWD/dist:/out:Z" archlinux:base-devel bash -c '
    pacman -Syu --noconfirm cargo libxkbcommon wayland libx11 vulkan-icd-loader >/dev/null &&
    cp -r /work /build && useradd -m builder && chown -R builder /build &&
    su builder -c "cd /build && makepkg -f --noconfirm" &&
    cp /build/*.pkg.tar.zst /out/ && chown --reference=/out /out/*.pkg.tar.zst'
  rm -rf "$work"
}

windows() {
  if [ "$(uname -s)" = Linux ]; then
    need cargo-xwin "cargo install cargo-xwin  (e: rustup target add $WIN)"
    cargo xwin build --release --locked --target "$WIN"
  else
    rustup target add "$WIN"
    cargo build --release --locked --target "$WIN"
  fi
  cp "target/$WIN/release/pdrop.exe" "dist/pdrop-${VERSION}-windows-x86_64.exe"
}

msi() {
  windows
  local exe out
  exe="$PWD/target/$WIN/release/pdrop.exe"
  out="dist/pdrop-${VERSION}-x86_64.msi"
  if command -v wixl >/dev/null; then # Linux: msitools
    wixl -D Version="$VERSION" -D Exe="$exe" --arch x64 -o "$out" packaging/windows/pdrop.wxs
  elif command -v candle >/dev/null || command -v candle.exe >/dev/null; then # Windows: WiX 3
    mkdir -p target/wix
    candle -nologo -arch x64 -dVersion="$VERSION" -dExe="$exe" -out target/wix/pdrop.wixobj packaging/windows/pdrop.wxs
    light -nologo -out "$out" target/wix/pdrop.wixobj
  else
    echo "faltando wixl (Linux: sudo dnf install msitools) ou WiX 3 (Windows: candle/light no PATH)" >&2
    exit 1
  fi
}

# macOS: binário universal (Apple Silicon + Intel). Só roda num Mac: precisa do SDK da Apple.
macos() {
  [ "$(uname -s)" = Darwin ] || { echo "o pacote do macOS só builda num Mac (ou no runner macOS do CI)" >&2; exit 1; }
  rustup target add aarch64-apple-darwin x86_64-apple-darwin
  cargo build --release --locked --target aarch64-apple-darwin
  cargo build --release --locked --target x86_64-apple-darwin
  mkdir -p target/universal
  lipo -create -output target/universal/pdrop target/aarch64-apple-darwin/release/pdrop target/x86_64-apple-darwin/release/pdrop
  tar czf "dist/pdrop-${VERSION}-macos-universal.tar.gz" -C target/universal pdrop -C ../.. LICENSE README.md
}

# troca a versão em todos os arquivos que a repetem
version() {
  local v=${1:?uso: package.sh version X.Y.Z}
  sed -i "0,/^version = \".*\"/s//version = \"$v\"/" Cargo.toml
  sed -i "s/^pkgver=.*/pkgver=$v/; s/^pkgrel=.*/pkgrel=1/" packaging/arch/PKGBUILD
  sed -i "s/^Version:.*/Version:        $v/" packaging/rpm/pdrop.spec
  sed -i "s/^version: .*/version: '$v'/" snap/snapcraft.yaml
  sed -i "s|/tags/v[0-9.]*\.tar\.gz|/tags/v$v.tar.gz|" packaging/homebrew/pdrop.rb
  cargo update --workspace --quiet
  echo "versão $v: confira 'git diff' e crie a tag v$v"
}

[ $# -ge 1 ] || { sed -n '2,3p' "$0"; exit 1; }
cmd=$1; shift
case $cmd in
  linux) deb; rpm ;;
  deb | rpm | arch | windows | msi | macos | version) "$cmd" "$@" ;;
  *) echo "pacote desconhecido: $cmd" >&2; exit 1 ;;
esac
ls -lh dist
