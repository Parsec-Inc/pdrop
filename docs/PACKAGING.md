# Empacotamento e releases

Tudo sai de `scripts/package.sh`, o mesmo script no seu computador e no CI.

## Build local

| Comando | Gera (em `dist/`) | Precisa de |
|---|---|---|
| `scripts/package.sh deb` | `.deb` | `cargo install cargo-deb` |
| `scripts/package.sh rpm` | `.rpm` | `cargo install cargo-generate-rpm` |
| `scripts/package.sh arch` | `.pkg.tar.zst` | podman ou docker (roda `makepkg` num container Arch) |
| `scripts/package.sh windows` | `.exe` | Linux: `cargo install cargo-xwin` e `rustup target add x86_64-pc-windows-msvc` |
| `scripts/package.sh msi` | `.msi` | o de cima + `sudo dnf install msitools` (Linux) ou WiX 3 (Windows) |
| `scripts/package.sh macos` | `.tar.gz` universal | **só num Mac** (SDK da Apple) |
| `scripts/package.sh linux` | deb + rpm | |

O snap e o Homebrew não têm comando local: o snap builda com `snapcraft` (na raiz do repo), e a fórmula
do Homebrew compila o código-fonte na máquina de quem instala (`brew install --build-from-source ./packaging/homebrew/pdrop.rb`).

## Fazer uma release

```bash
scripts/package.sh version 0.2.0   # atualiza Cargo.toml, PKGBUILD, .spec, snapcraft.yaml e a fórmula
git commit -am "release 0.2.0" && git tag v0.2.0 && git push --follow-tags
```

O push da tag roda `.github/workflows/release.yml`: builda deb, rpm, Arch, MSI, macOS e snap, e
anexa tudo numa release do GitHub. O job falha se a tag não bater com a versão do `Cargo.toml`.

## Canais de instalação (configuração única)

Estes passos exigem contas suas; nada disso foi feito ainda.

| Canal | Instalação para o usuário | O que configurar |
|---|---|---|
| **GitHub Releases** | baixar `.deb` / `.rpm` / `.msi` / `.tar.gz` | nada |
| **dnf / COPR** | `sudo dnf copr enable parsec-inc/pdrop && sudo dnf install pdrop` | Crie o projeto em copr.fedorainfracloud.org, em *Packages > New* escolha *SCM*, repo `Parsec-Inc/pdrop`, spec `packaging/rpm/pdrop.spec`. Nas configurações do projeto, ligue **Enable internet access during builds** (o cargo baixa as dependências). Para buildar a cada tag, adicione o webhook do COPR ao GitHub. |
| **pacman / AUR** | `yay -S pdrop` | Crie o pacote `pdrop` no AUR com a sua chave SSH. No GitHub, crie o secret `AUR_SSH_PRIVATE_KEY` e a variável `PUBLISH_AUR=true`. |
| **snap** | `sudo snap install pdrop` | `snapcraft register pdrop`, depois `snapcraft export-login` e guarde no secret `SNAPCRAFT_STORE_CREDENTIALS`. Variável `PUBLISH_SNAP=true`. Publica no canal `edge`. |
| **brew** | `brew install Parsec-Inc/tap/pdrop` | Crie o repo `Parsec-Inc/homebrew-tap`, copie `packaging/homebrew/pdrop.rb` para `Formula/pdrop.rb` (com o `sha256` real). Secret `HOMEBREW_TAP_TOKEN` (PAT com escrita no tap) e variável `PUBLISH_BREW=true`. |
| **apt** | `sudo apt install ./pdrop_*.deb` | Um repositório apt de verdade (PPA, ou um repo estático no GitHub Pages) não está montado. Por enquanto o `.deb` vem da release. |
| **msi** | duplo clique | Sem assinatura de código: o Windows SmartScreen vai avisar. |

AUR, COPR e Homebrew baixam o tarball da tag no GitHub, então o repositório precisa ser **público** para eles funcionarem.

## O que não foi testado

- `.deb`, `.rpm`, pacote Arch e `.msi` foram gerados e inspecionados (conteúdo, metadados, dependências), mas **nenhum foi instalado**.
- O `.exe` e o MSI nunca rodaram num Windows. A ponte de clipboard no Windows usa PowerShell e também não foi exercitada.
- O macOS nunca foi compilado (precisa de um Mac ou do runner do CI).
- O `snapcraft.yaml` nunca foi buildado (`strict` com o plug `home`); o CI será o primeiro build real.
- Os workflows nunca rodaram.
