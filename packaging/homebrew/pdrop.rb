# Fórmula para o tap Parsec-Inc/homebrew-tap (copie para Formula/pdrop.rb).
# A release do CI atualiza url e sha256 sozinha; veja docs/PACKAGING.md.
class Pdrop < Formula
  desc "AirDrop-style sharing of files, text and clipboard on your local network"
  homepage "https://github.com/Parsec-Inc/pdrop"
  url "https://github.com/Parsec-Inc/pdrop/archive/refs/tags/v1.2.1.tar.gz"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  license "MIT"
  head "https://github.com/Parsec-Inc/pdrop.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "uso:", shell_output("#{bin}/pdrop --help 2>&1")
  end
end
