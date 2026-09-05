# Formula for a Homebrew tap. Copy to reloading01/homebrew-tap as
# Formula/certstream-server-rust.rb; the release workflow rewrites the
# version and the four sha256 lines on every tag.
class CertstreamServerRust < Formula
  desc "Certificate Transparency log streaming server"
  homepage "https://certstream.dev"
  version "1.6.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/reloading01/certstream-server-rust/releases/download/v#{version}/certstream-server-rust-#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "REPLACE_AARCH64_APPLE_DARWIN"
    end
    on_intel do
      url "https://github.com/reloading01/certstream-server-rust/releases/download/v#{version}/certstream-server-rust-#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "REPLACE_X86_64_APPLE_DARWIN"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/reloading01/certstream-server-rust/releases/download/v#{version}/certstream-server-rust-#{version}-aarch64-unknown-linux-musl.tar.gz"
      sha256 "REPLACE_AARCH64_UNKNOWN_LINUX_MUSL"
    end
    on_intel do
      url "https://github.com/reloading01/certstream-server-rust/releases/download/v#{version}/certstream-server-rust-#{version}-x86_64-unknown-linux-musl.tar.gz"
      sha256 "REPLACE_X86_64_UNKNOWN_LINUX_MUSL"
    end
  end

  def install
    bin.install "certstream-server-rust"
    pkgshare.install "config.example.yaml"
    doc.install "README.md"
  end

  service do
    run [opt_bin/"certstream-server-rust"]
    keep_alive true
    log_path var/"log/certstream-server-rust.log"
    error_log_path var/"log/certstream-server-rust.log"
    environment_variables CERTSTREAM_CT_LOG_STATE_FILE: var/"lib/certstream/state.json"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/certstream-server-rust --version")
  end
end
