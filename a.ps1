
#$env:RUST_BACKTRACE=1; .\target\release\dirstat-rs.exe --log-modules pt 2>&1 | Tee-Object -FilePath profile.log
.\target\release\dirstat-rs.exe --log-modules pt 2>&1 | Tee-Object profile2.log | Select-String "upload_scene|WF dispatch|cache MISS"