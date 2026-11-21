# Template. packaging/update-homebrew.sh renders it into the tap's Formula/agentcarousel.rb
# for each release; the upper-case placeholders are filled from the release's SHA256SUMS.
# Prebuilt binaries only: no Rust toolchain is needed to install.
class Agentcarousel < Formula
  desc "Unit tests for AI agents: behavioral tests in CI, LLM-judge scoring, signed evidence"
  homepage "https://github.com/agentcarousel/agentcarousel"
  version "@VERSION@"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/agentcarousel/agentcarousel/releases/download/@TAG@/agentcarousel-@TAG@-aarch64-apple-darwin.tar.gz"
      sha256 "@SHA256_AARCH64_APPLE_DARWIN@"
    end
    on_intel do
      url "https://github.com/agentcarousel/agentcarousel/releases/download/@TAG@/agentcarousel-@TAG@-x86_64-apple-darwin.tar.gz"
      sha256 "@SHA256_X86_64_APPLE_DARWIN@"
    end
  end

  # Linux uses the static musl builds: they run on any distribution, whatever its glibc.
  on_linux do
    on_arm do
      url "https://github.com/agentcarousel/agentcarousel/releases/download/@TAG@/agentcarousel-@TAG@-aarch64-unknown-linux-musl.tar.gz"
      sha256 "@SHA256_AARCH64_UNKNOWN_LINUX_MUSL@"
    end
    on_intel do
      url "https://github.com/agentcarousel/agentcarousel/releases/download/@TAG@/agentcarousel-@TAG@-x86_64-unknown-linux-musl.tar.gz"
      sha256 "@SHA256_X86_64_UNKNOWN_LINUX_MUSL@"
    end
  end

  def install
    bin.install "agentcarousel"
    bin.install_symlink "agentcarousel" => "agc"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/agentcarousel --version")
  end
end
