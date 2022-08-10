import esbuild from "esbuild";
import { sassPlugin } from "esbuild-sass-plugin";

let watch = null;
if (process.argv[2] === "watch") {
  watch = {
    onRebuild(err) {
      if (err) {
        console.error("watch build failed:", err);
      } else {
        console.log("watch build succeeded");
      }
    },
  };
}

esbuild
  .build({
    entryPoints: ["js/app.tsx"],
    bundle: true,
    sourcemap: true,
    target: "es6",
    plugins: [sassPlugin()],
    outfile: "dist/js/app.js",
    watch,
  })
  .then(() => {
    console.log("esbuild succeeded");
  })
  .catch((err) => {
    console.error(err);
    process.exit(1);
  });
