const { app, BrowserWindow, Menu, shell } = require('electron');
const path = require('path');
function createWindow() {
  const win = new BrowserWindow({ width: 1200, height: 850, title: "nova-launcher", autoHideMenuBar: true, webPreferences: { contextIsolation: true, nodeIntegration: false } });
  Menu.setApplicationMenu(null);
  win.webContents.setWindowOpenHandler(({ url }) => { if (/^https?:/i.test(url)) shell.openExternal(url); return { action: 'deny' }; });
  win.webContents.on('will-navigate', (e, url) => { if (/^https?:/i.test(url)) { e.preventDefault(); shell.openExternal(url); } });
  win.loadFile(path.join(__dirname, 'site', "src/index.html"));
}
app.whenReady().then(createWindow);
app.on('window-all-closed', () => app.quit());
