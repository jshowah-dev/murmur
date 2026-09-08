@echo off
setlocal
set DEST=%LOCALAPPDATA%\Murmur\models
set MODEL=sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8
set BASE=https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models
set CURL=%SystemRoot%\System32\curl.exe
set TAR=%SystemRoot%\System32\tar.exe
if not exist "%DEST%" mkdir "%DEST%"
if not exist "%DEST%\silero_vad.onnx" (
  echo Downloading silero_vad.onnx ...
  "%CURL%" -SL -o "%DEST%\silero_vad.onnx" "%BASE%/silero_vad.onnx" || exit /b 1
)
if not exist "%DEST%\%MODEL%\encoder.int8.onnx" (
  if not exist "%DEST%\%MODEL%.tar.bz2" (
    echo Downloading %MODEL% ~500 MB ...
    "%CURL%" -SL -o "%DEST%\%MODEL%.tar.bz2" "%BASE%/%MODEL%.tar.bz2" || exit /b 1
  )
  "%TAR%" -xjf "%DEST%\%MODEL%.tar.bz2" -C "%DEST%" || exit /b 1
  del "%DEST%\%MODEL%.tar.bz2"
)
echo Done. Model files:
dir "%DEST%\%MODEL%"
