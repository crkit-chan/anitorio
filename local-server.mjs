import fs from "node:fs";
import https from "node:https";
import path from "node:path";

const root = path.resolve("dist");
const base = "/anitorio";

https
  .createServer(
    {
      key: fs.readFileSync("key.pem"),
      cert: fs.readFileSync("cert.pem"),
    },
    (req, res) => {
      const pathname = decodeURIComponent(new URL(req.url, "https://localhost").pathname);

      if (pathname !== base && !pathname.startsWith(base + "/")) {
        res.writeHead(404).end();
        return;
      }

      const relative = pathname.slice(base.length).replace(/^\/+/, "");
      let file = path.resolve(root, relative || "index.html");

      if (!file.startsWith(root + path.sep) && file !== path.join(root, "index.html")) {
        res.writeHead(403).end();
        return;
      }

      if (fs.existsSync(file) && fs.statSync(file).isDirectory()) {
        file = path.join(file, "index.html");
      }

      if (!fs.existsSync(file) || !fs.statSync(file).isFile()) {
        res.writeHead(404).end();
        console.log("404:", pathname, "->", file);
        return;
      }

      res.writeHead(200);
      fs.createReadStream(file).pipe(res);
    },
  )
  .listen(4173, "0.0.0.0", () => {
    console.log("https://localhost:4173/anitorio/");
  });
