# typed: false
# frozen_string_literal: true

# Homebrew formula for melos-rs
#
# Install:
#   brew tap pastel-sketchbook/melos-rs https://github.com/pastel-sketchbook/melos-rs
#   brew install melos-rs
#
# Or directly:
#   brew install pastel-sketchbook/melos-rs/melos-rs
#
# After a new release, update the version, URL tags, and sha256 values below.
# SHA256 values are in the .sha256 files attached to each GitHub release.

class MelosRs < Formula
  desc "Fast Rust replacement for Melos — Flutter/Dart monorepo management"
  homepage "https://github.com/pastel-sketchbook/melos-rs"
  version "0.6.7"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/pastel-sketchbook/melos-rs/releases/download/v#{version}/melos-rs-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "3907d3b1787fca926a604e1b424924c725f8e6299f03697a61291493e830bfed"
    end

    on_intel do
      url "https://github.com/pastel-sketchbook/melos-rs/releases/download/v#{version}/melos-rs-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "29eaf67f6196b5ad527ea0f40e3b1135de280eac2a54c7d489e6a8a3241b6662"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/pastel-sketchbook/melos-rs/releases/download/v#{version}/melos-rs-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "07b556a39c7d74ccdf78b1d8f42894964ec0c94451843a113875e1e6cf255805"
    end

    on_intel do
      url "https://github.com/pastel-sketchbook/melos-rs/releases/download/v#{version}/melos-rs-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "b561ec03cc0b3e305e2aacfdbdc54025bc2805383878912765594b3e9bd01c4f"
    end
  end

  def install
    bin.install "melos-rs"
  end

  test do
    assert_match "melos-rs", shell_output("#{bin}/melos-rs --version")
  end
end
