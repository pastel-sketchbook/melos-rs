# typed: false
# frozen_string_literal: true

# Homebrew formula for melos-tui
#
# Install:
#   brew tap pastel-sketchbook/melos-rs https://github.com/pastel-sketchbook/melos-rs
#   brew install melos-tui
#
# After a new release, update the version, URL tags, and sha256 values below.

class MelosTui < Formula
  desc "Terminal UI for melos-rs — Flutter/Dart monorepo management"
  homepage "https://github.com/pastel-sketchbook/melos-rs"
  version "0.7.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/pastel-sketchbook/melos-rs/releases/download/v#{version}/melos-tui-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "b6b19078e512b8de02177d102b30c030db6adcc6d46b9fd18604aa4787e8f0d3"
    end

    on_intel do
      url "https://github.com/pastel-sketchbook/melos-rs/releases/download/v#{version}/melos-tui-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "d96d2a5500bd1997cbbf7175bcbd7f8baa7ca20fb361cb505520f72c36aa783d"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/pastel-sketchbook/melos-rs/releases/download/v#{version}/melos-tui-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "4dca06ebb19335aebc5e053eba5e508294ec7bca0fd16bde00257aee157e23d9"
    end

    on_intel do
      url "https://github.com/pastel-sketchbook/melos-rs/releases/download/v#{version}/melos-tui-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "8c8d509b739f85ff514d3e9b6085cffb4dd11d338bc51a7de851853cb85ac52b"
    end
  end

  def install
    bin.install "melos-tui"
  end

  test do
    # melos-tui requires a terminal, so just check it exists
    assert_predicate bin/"melos-tui", :executable?
  end
end
