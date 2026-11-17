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

Não tem passo manual: todo push na `main` roda `.github/workflows/release.yml`. Os testes não
rodam de novo aqui: eles já passaram no PR (`ci.yml`, testes e build em Linux, macOS e Windows,
cancelando o run anterior quando entra um push novo no mesmo PR).

1. **bump**: o commitizen (`.cz.toml`) calcula a versão pelos commits desde a última tag
   (`feat` sobe minor, `fix` sobe patch, `!` ou `BREAKING CHANGE` sobe major), troca a
   versão no `Cargo.toml`, `Cargo.lock`, PKGBUILD, `.spec`, `snapcraft.yaml` e na fórmula, escreve o
   `CHANGELOG.md`, cria a tag `vX.Y.Z` e dá push. Sem `feat`/`fix` desde a última tag, para aqui.
2. **builds**: deb, rpm, Arch, MSI, macOS e snap, todos da tag nova.
3. **release**: anexa tudo, com `SHA256SUMS` e as notas do CHANGELOG. Só o linux é obrigatório:
   se outra plataforma falhar, a release sai sem ela.
4. **aur**, **homebrew** e o **snap** na loja: só com as variáveis ligadas (abaixo).

Para forçar uma versão, rode o workflow **release** à mão (*Actions > release > Run workflow*) e
escolha o incremento. Os commits precisam seguir o Conventional Commits (`feat:`, `fix:`, `ci:`...).

## Firewall

Os pacotes não mexem no firewall. O pdrop confere sozinho (firewalld, ufw, Firewall do Windows e
do macOS) e, se algo parece bloquear, mostra uma faixa no topo com o comando para liberar; clicar
copia. Quem decide é o usuário.

Os pacotes de Linux trazem o serviço `pdrop` do firewalld (`/usr/lib/firewalld/services/pdrop.xml`)
e o perfil `pdrop` do ufw (`/etc/ufw/applications.d/pdrop`), com UDP 47474 (descoberta) e TCP 47475
(transferência). Eles não ficam ativos sozinhos: `sudo firewall-cmd --permanent --add-service=pdrop`
ou `sudo ufw allow pdrop`. Se a porta 47475 estiver ocupada, o pdrop usa uma porta aleatória, que
essas regras não cobrem.

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

- `.deb`, `.rpm` e o pacote Arch foram gerados; `.deb` e `.rpm` foram instalados e removidos em containers (Debian e Fedora);
  o serviço do firewalld passou no `firewall-offline-cmd`.
- O aviso de firewall no app compilou para Linux e Windows, mas não foi visto com um firewall bloqueando de verdade.
- O `.exe` e o MSI nunca rodaram num Windows. A ponte de clipboard no Windows usa PowerShell e também não foi exercitada.
- O macOS ainda não compilou no CI (o job foi cancelado na v0.1.2).
- O fluxo novo de release (bump na `main`) só rodam no CI.
