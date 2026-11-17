# Empacotamento e releases

Tudo sai de `scripts/package.sh`, o mesmo script no seu computador e no CI.

## Build local

| Comando | Gera (em `dist/`) | Precisa de |
|---|---|---|
| `scripts/package.sh deb` | `.deb` | `cargo install cargo-deb` |
| `scripts/package.sh rpm` | `.rpm` | `cargo install cargo-generate-rpm` |
| `scripts/package.sh arch` | `.pkg.tar.zst` | podman ou docker (roda `makepkg` num container Arch) |
| `scripts/package.sh windows` | `.exe` | Linux: `cargo install cargo-xwin` e `rustup target add x86_64-pc-windows-msvc` |
| `scripts/package.sh msi` | `.msi` (com exceção no firewall) | o de cima + WiX 3 (só Windows: a extensão de firewall não existe no `wixl`) |
| `scripts/package.sh macos` | `.tar.gz` universal | **só num Mac** (SDK da Apple) |
| `scripts/package.sh linux` | deb + rpm | |

O snap e o Homebrew não têm comando local: o snap builda com `snapcraft` (na raiz do repo), e a fórmula
do Homebrew compila o código-fonte na máquina de quem instala (`brew install --build-from-source ./packaging/homebrew/pdrop.rb`).

## Fazer uma release

Não tem passo manual: todo push na `main` roda `.github/workflows/release.yml`.

1. **tests**: o `ci.yml` (testes e build em Linux, macOS e Windows).
2. **bump**: o commitizen (`.cz.toml`) calcula a versão pelos commits desde a última tag
   (`feat` sobe minor, `fix` sobe patch; com `major_version_zero`, nada vira 1.0 sozinho), troca a
   versão no `Cargo.toml`, `Cargo.lock`, PKGBUILD, `.spec`, `snapcraft.yaml` e na fórmula, escreve o
   `CHANGELOG.md`, cria a tag `vX.Y.Z` e dá push. Sem `feat`/`fix` desde a última tag, para aqui.
3. **builds**: deb, rpm, Arch, MSI, macOS e snap, todos da tag nova.
4. **release**: anexa tudo, com `SHA256SUMS` e as notas do CHANGELOG. Só o linux é obrigatório:
   se outra plataforma falhar, a release sai sem ela.
5. **aur**, **homebrew** e o **snap** na loja: só com as variáveis ligadas (abaixo).

Para forçar uma versão, rode o workflow **release** à mão (*Actions > release > Run workflow*) e
escolha o incremento. Os commits precisam seguir o Conventional Commits (`feat:`, `fix:`, `ci:`...).

## Permissões que os pacotes configuram

| Onde | O quê |
|---|---|
| Linux (deb, rpm, Arch) | serviço `pdrop` do firewalld e perfil do ufw: UDP 47474 (descoberta) e TCP 47475 (transferência). Ao instalar, `/usr/share/pdrop/firewall open` libera no firewall ativo; ao remover, `close` fecha |
| Windows (MSI) | exceção do Firewall do Windows para o `pdrop.exe` |
| snap | os plugs `network` e `network-bind` |
| macOS | nada: o firewall do macOS pergunta na primeira vez |

Se a porta 47475 estiver ocupada, o pdrop usa uma porta aleatória, que o firewall não cobre.

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

- `.deb`, `.rpm` e o pacote Arch foram gerados; `.deb` e `.rpm` foram instalados e removidos em containers (Debian e Fedora), com os scripts de
  firewall; o serviço do firewalld passou no `firewall-offline-cmd`. A abertura de portas com
  firewalld/ufw rodando de verdade não foi exercitada.
- O `.exe` e o MSI nunca rodaram num Windows. A ponte de clipboard no Windows usa PowerShell e também não foi exercitada.
- O macOS ainda não compilou no CI (o job foi cancelado na v0.1.2).
- O MSI com a extensão de firewall e o fluxo novo de release (bump na `main`) só rodam no CI.
