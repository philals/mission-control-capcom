// A story's plan (story.md) rendered as markdown. Opened beside the board with props { file, title }.

import { View, div } from "gpui-kit";
import { v_flex, h_flex, TextView } from "gpui-base";
import { props, theme, readTextFile } from "herdr";

const EVERY_MS = 3000;

export default class CapcomPlan extends View {
  init(_props, cx) {
    const p = props() ?? {};
    this.file = p.file ?? null;
    this.title = p.title ?? "Plan";
    this.text = null;
    this.error = this.file ? null : "No plan file was given";
    if (this.file) {
      cx.timer.every(EVERY_MS, (cx) => this.load(cx));
      this.load(cx);
    }
  }

  async load(cx) {
    try {
      const text = await readTextFile(this.file);
      if (text === null) {
        this.error = `${this.file} does not exist yet. Run /story-plan for this story.`;
        this.text = null;
      } else {
        this.error = null;
        if (text === this.text) return;
        this.text = text;
      }
    } catch (e) {
      this.error = String(e?.message ?? e);
    }
    cx.notify();
  }

  render(_cx) {
    const t = theme();
    return v_flex().id("plan-root").size_full().bg(t.surface)
      .child(
        h_flex().px(10).py(6).flex_none().gap(6).items_center()
          .child(div().text_size(12).font_semibold().text_color(t.foreground).child(this.title))
          .child(div().flex_1().min_w(0).text_size(10).text_color(t.dim).child(this.file ?? "")),
      )
      .child(
        div().id("plan-body").flex_1().min_h(0).px(12).pb(12).overflow_y_scrollbar().text_color(t.foreground)
          .child(
            this.error
              ? div().text_size(12).text_color("#ffb000").child(this.error)
              : TextView.markdown("plan-text", this.text ?? "").selectable(true),
          ),
      );
  }
}
