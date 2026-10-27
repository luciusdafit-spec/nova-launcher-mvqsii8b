# Nova Launcher (Tauri 2 + Rust)

Minecraft launcher ที่ติดตั้งผ่านเว็บ (NSIS installer เล็ก + โหลด WebView2 เองถ้าไม่มี) และอัปเดตตัวเองอัตโนมัติ

## ตั้งค่าครั้งแรก
1. `npm install` แล้ว `npx tauri signer generate -w nova.key` → ได้ public key + private key
2. วาง **public key** ใน `src-tauri/tauri.conf.json` → `plugins.updater.pubkey`
3. แก้ `OWNER/REPO` ใน `tauri.conf.json` (endpoints) และ `web/index.html`
4. GitHub → Settings → Secrets: `TAURI_SIGNING_PRIVATE_KEY` (เนื้อไฟล์ nova.key), `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
   (**ห้าม commit nova.key**; ถ้าทำหาย จะออกอัปเดตให้คนที่ติดตั้งไปแล้วไม่ได้)
5. ทดสอบ: `npx tauri dev`

## ออกเวอร์ชันใหม่ (ผู้เล่นได้อัปเดตเองตอนเปิด launcher)
1. เพิ่ม `version` ใน `src-tauri/tauri.conf.json` และ `Cargo.toml`
2. `git tag v0.1.1 && git push --tags` → GitHub Actions build, เซ็น, สร้าง `latest.json` ให้เอง

## เว็บดาวน์โหลด
อัปโหลดโฟลเดอร์ `web/` ขึ้น GitHub Pages / Cloudflare Pages / โฮสต์ใดก็ได้ — ปุ่มจะดึง `-setup.exe` ล่าสุดจาก Releases เอง

## อัปเดต mod/config ของเซิร์ฟเวอร์
ตั้ง `PACK_URL` ใน `src/index.html` ชี้ไป `pack.json`:
`{"version":"1.0","files":[{"path":"mods/a.jar","url":"https://...","sha1":"..."}],"delete":["mods/old.jar"]}`
(ต้องมี sha1 ทุกไฟล์ และใช้ HTTPS เท่านั้น)

## ที่ยังไม่ได้ทำ
- Microsoft login (ตอนนี้ใช้ offline profile ใน `launch`, token เป็น placeholder) — ต้องสร้าง Azure app ของตัวเอง
- Forge/Fabric/NeoForge, รองรับเวอร์ชัน < 1.19 (natives แบบเก่า), Mac/Linux
