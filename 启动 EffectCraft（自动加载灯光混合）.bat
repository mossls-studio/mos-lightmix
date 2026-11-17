@echo off
rem 以「自动加载外部效果」模式启动 EffectCraft，mos_灯光混合 会直接出现在效果面板 MOS 分类里
set EFFECTCRAFT_HELPERS=1
if exist "C:\Program Files\EffectCraft\effectcraft.exe" (
    start "" "C:\Program Files\EffectCraft\effectcraft.exe"
) else (
    echo 找不到 C:\Program Files\EffectCraft\effectcraft.exe，请修改本文件里的路径
    pause
)
