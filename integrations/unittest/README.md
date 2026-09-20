# Upstream unittest capture

`capture_unittest.py` is copied without source changes from CortexWeave revision
`754126bfc2efd6330253826c89922148d33d9915`, path
`integrations/unittest/capture_unittest.py`. Its Apache 2.0 license is included
in `LICENSE`. The copied file's SHA-256 is
`5e030dcb34aae283b8b8bbc29870059f9daddbad76dce6e10b79a83aa286bf72`.

`shuttle_capture.py` is Shuttle's wrapper. It redirects test chatter to bounded
stderr and combines the upstream normalized bundle and raw artifact into stdout
for the existing process executor. It does not reimplement unittest callbacks.
The native qualified profile requires Python 3.14.7. No packages are installed by
either script. Copy both scripts into the isolated qualification workspace and
declare them as runner configuration inputs. See
[producer qualification](../../docs/evidence-qualification.md).
