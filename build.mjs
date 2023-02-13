import esbuild from "esbuild";
import { sassPlugin } from "esbuild-sass-plugin";

const context = await esbuild.context({
    entryPoints: ["js/app.ts", "js/firehose.ts"],
    bundle: true,
    sourcemap: true,
    target: "es6",
    plugins: [sassPlugin()],
    outdir: "dist/js",
    logLevel: "info",
});

if (process.argv[2] === "watch") {
    await context.watch();
    // TODO: Is it better to serve here than from Rust now that it's possible?
} else {
    await context.rebuild();
    console.log("esbuild finished");

    await context.dispose();
}
