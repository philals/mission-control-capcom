// A story's plan (story.md) rendered as markdown. Opened beside the board with props { file, title }.

import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex, TextView } from "gpui-base";
import { props, theme, readTextFile, run, focused, panes } from "herdr";

const EVERY_MS = 3000;

export default class CapcomPlan extends View {
  init(_props, cx) {
    const p = props() ?? {};
    this.file = p.file ?? null;
    this.title = p.title ?? "Plan";
    this.handle = cx.focus_handle();
    this.handle.focus();
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

  close() {
    const pane = props()?.pane ?? focused()?.pane ?? panes().find((p) => p.focused)?.pane;
    if (pane) run("pane.close", { pane });
  }

  render(_cx) {
    const t = theme();
    return v_flex().id("plan-root").track_focus(this.handle).size_full().bg(t.surface)
      .on_mouse_down("left", () => this.handle.focus())
      .on_key_down((e) => { if (e.key === "escape") this.close(); })
      .child(
        h_flex().px(10).py(6).flex_none().gap(6).items_center()
          .child(div().text_size(12).font_semibold().text_color(t.foreground).child(this.title))
          .child(div().flex_1().min_w(0).text_size(10).text_color(t.dim).child(this.file ?? ""))
          .child(
            div().id("plan-close").flex_none().p(3).rounded(4).cursor_pointer().text_color(t.dim)
              .hover((s) => s.bg(t.hover).text_color(t.foreground)).tooltip("Close the plan")
              .on_click(() => this.close())
              .child(svg("mods/pane/close.svg").size(13)),
          ),
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
