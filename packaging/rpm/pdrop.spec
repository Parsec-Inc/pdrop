# Para o COPR (build a partir do código-fonte). Ative "Enable internet access during
# this build" nas configurações do projeto no COPR: o cargo baixa as dependências.
%global debug_package %{nil}

Name:           pdrop
Version:        2.0.1
Release:        1%{?dist}
Summary:        AirDrop-style sharing of files, text and clipboard on your local network
License:        MIT
URL:            https://github.com/Parsec-Inc/pdrop
Source0:        %{url}/archive/v%{version}/%{name}-%{version}.tar.gz

BuildRequires:  cargo >= 1.85
BuildRequires:  gcc
Recommends:     wl-clipboard
Recommends:     libnotify

%description
Run pdrop on two or more machines on the same network and they appear on each
other's radar. Select a peer and drop files, text or your clipboard to them.
No accounts, no servers, no cloud: everything travels directly over the LAN.

%prep
%autosetup

%build
cargo build --release --locked

%install
install -Dm755 target/release/pdrop %{buildroot}%{_bindir}/pdrop
install -Dm644 assets/pdrop.desktop %{buildroot}%{_datadir}/applications/pdrop.desktop
install -Dm644 assets/pdrop.png %{buildroot}%{_datadir}/icons/hicolor/256x256/apps/pdrop.png
install -Dm644 packaging/linux/pdrop.xml %{buildroot}%{_prefix}/lib/firewalld/services/pdrop.xml
install -Dm644 packaging/linux/ufw-pdrop %{buildroot}%{_sysconfdir}/ufw/applications.d/pdrop

%files
%license LICENSE
%doc README.md
%{_bindir}/pdrop
%{_datadir}/applications/pdrop.desktop
%{_datadir}/icons/hicolor/256x256/apps/pdrop.png
%{_prefix}/lib/firewalld/services/pdrop.xml
%config(noreplace) %{_sysconfdir}/ufw/applications.d/pdrop

%changelog
* Tue Nov 17 2026 Parsec-Inc <noreply@github.com> - 0.1.0-1
- Primeiro pacote
