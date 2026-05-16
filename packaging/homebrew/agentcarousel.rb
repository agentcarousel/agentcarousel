class Agentcarousel < Formula
  desc "Unit tests for AI agents — define behaviour in YAML, run offline, export signed evidence bundles"
  homepage "https://github.com/agentcarousel/agentcarousel"
  url "https://github.com/agentcarousel/agentcarousel/archive/refs/tags/v0.4.8.tar.gz"
  sha256 "<sha256-updated-by-packaging/update-homebrew.sh>"
  license "MIT"
  head "https://github.com/agentcarousel/agentcarousel.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/agentcarousel")
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/agentcarousel --version")
  end
end
