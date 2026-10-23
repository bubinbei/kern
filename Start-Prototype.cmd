@echo off
cd /d "%~dp0dist\Kern-Prototype-0.1.0"
if errorlevel 1 exit /b 1
Start-Kern.cmd
