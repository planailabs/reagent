import MarkdownIt from "markdown-it";

// html: false escapes raw HTML and markdown-it refuses javascript:/data: links,
// so its output is safe for v-html.
const md = new MarkdownIt({ html: false, linkify: true, breaks: true });
const link = md.renderer.rules.link_open ?? ((tokens, i, options, env, self) => self.renderToken(tokens, i, options));
md.renderer.rules.link_open = (tokens, i, options, env, self) => {
  tokens[i].attrSet("target", "_blank");
  tokens[i].attrSet("rel", "noopener noreferrer");
  return link(tokens, i, options, env, self);
};

/** Markdown as HTML. */
export const markdown = (text) => md.render(text || "");

/** Markdown as plain text, for where nothing renders it (system notifications). */
export function plain(text) {
  const out = [];
  for (const b of md.parse(text || "", {})) {
    if (b.type === "fence" || b.type === "code_block") out.push(b.content.replace(/\n$/, ""), "\n");
    else if (b.type === "inline")
      for (const t of b.children)
        out.push(t.type === "softbreak" || t.type === "hardbreak" ? "\n" : t.type === "image" ? t.content : t.content);
    else if (b.type === "paragraph_close") out.push(b.hidden ? "\n" : "\n\n");
    else if (b.type.endsWith("_close") && !["list_item_close", "bullet_list_close", "ordered_list_close"].includes(b.type)) out.push("\n");
    else if (b.type === "list_item_open") out.push(b.info ? `${b.info}. ` : "• ");
  }
  return out.join("").replace(/\n{3,}/g, "\n\n").trim();
}
