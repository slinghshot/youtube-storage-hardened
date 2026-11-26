"""Desktop frontend embedded in the youtube-storage executable."""
import os
import queue
import subprocess
import sys
import threading
import tkinter as tk
from pathlib import Path
from tkinter import filedialog, messagebox, ttk


class BackupForm(ttk.Frame):
    def __init__(self, parent, create, start):
        super().__init__(parent, padding=20)
        self.create = create
        self.start = start
        self.columnconfigure(1, weight=1)
        self.source = tk.StringVar()
        self.output = tk.StringVar()
        self.audio = tk.StringVar()
        self.secret = tk.StringVar()
        self.confirm = tk.StringVar()
        self.field(0, "Source" if create else "Backup video", self.source)
        picks = ttk.Frame(self)
        picks.grid(row=0, column=2, padx=(8, 0))
        ttk.Button(picks, text="File…", command=self.pick_file).pack(side="left")
        if create:
            ttk.Button(picks, text="Folder…", command=self.pick_folder).pack(side="left")
        self.field(1, "Output video" if create else "New restore folder", self.output)
        ttk.Button(self, text="Choose…", command=self.pick_output).grid(row=1, column=2, sticky="w", padx=8)
        if create:
            self.field(2, "Soundtrack (optional)", self.audio)
            ttk.Button(self, text="Choose…", command=self.pick_audio).grid(row=2, column=2, sticky="w", padx=8)
        self.field(3, "Passphrase", self.secret, masked=True)
        if create:
            self.field(4, "Confirm passphrase", self.confirm, masked=True)
        self.action = ttk.Button(self, text="Create backup" if create else "Restore files", command=self.submit)
        self.action.grid(row=5, column=1, sticky="w", pady=(16, 0))

    def field(self, row, title, variable, masked=False):
        ttk.Label(self, text=title).grid(row=row, column=0, sticky="w", padx=(0, 14), pady=7)
        ttk.Entry(self, textvariable=variable, show="*" if masked else "", width=42).grid(
            row=row, column=1, sticky="ew", pady=7)

    def select_source(self, value):
        if value:
            path = Path(value)
            self.source.set(value)
            self.output.set(str(path.with_suffix(".mp4") if self.create else
                                path.with_name(path.stem + "-restored")))

    def pick_file(self):
        self.select_source(filedialog.askopenfilename(title="Choose a file"))

    def pick_folder(self):
        self.select_source(filedialog.askdirectory(title="Choose a folder to back up"))

    def pick_output(self):
        if self.create:
            value = filedialog.asksaveasfilename(title="Choose a new output video", defaultextension=".mp4",
                                                  filetypes=[("MP4 video", "*.mp4")])
        else:
            parent = filedialog.askdirectory(title="Choose the parent of the new restore folder")
            value = str(Path(parent) / (Path(self.source.get()).stem + "-restored")) if parent else ""
        if value:
            self.output.set(value)

    def pick_audio(self):
        value = filedialog.askopenfilename(title="Choose a soundtrack",
            filetypes=[("Audio files", "*.wav *.mp3 *.flac *.m4a *.aac *.ogg"), ("All files", "*")])
        if value:
            self.audio.set(value)

    def submit(self):
        secret = self.secret.get()
        if not self.source.get() or not self.output.get() or not secret:
            messagebox.showerror("Missing information", "Choose the source and destination and enter a passphrase.")
            return
        if self.create and secret != self.confirm.get():
            messagebox.showerror("Passphrase mismatch", "Enter the same passphrase in both fields.")
            return
        if os.path.lexists(self.output.get()):
            messagebox.showerror("Destination exists", "Choose a new output path.")
            return
        args = ["compress" if self.create else "decompress", "--file", self.source.get(),
                "--output", self.output.get(), "--passphrase-stdin"]
        if self.create and self.audio.get():
            args.extend(["--audio", self.audio.get()])
        self.secret.set("")
        self.confirm.set("")
        self.start(args, secret, self.output.get())


def main(executable):
    root = tk.Tk()
    root.title("YouTube Storage")
    root.minsize(640, 330)
    outer = ttk.Frame(root, padding=16)
    outer.pack(expand=True, fill="both")
    notebook = ttk.Notebook(outer)
    notebook.pack(expand=True, fill="both")
    results = queue.Queue()
    status = tk.StringVar(value="")
    busy = False

    def finish():
        nonlocal busy
        try:
            error, destination = results.get_nowait()
        except queue.Empty:
            root.after(150, finish)
            return
        busy = False
        progress.stop()
        for form in forms:
            form.action.state(["!disabled"])
        status.set("Operation failed" if error else "Saved to " + destination)
        if error:
            messagebox.showerror("Backup operation failed", error)

    def start(args, secret, destination):
        nonlocal busy
        if busy:
            return
        busy = True
        for form in forms:
            form.action.state(["disabled"])
        status.set("Creating video…" if args[0] == "compress" else "Restoring files…")
        progress.start()

        def worker():
            try:
                result = subprocess.run([executable, *args], input=secret.encode("utf-8"),
                    stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                    creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
                error = "" if result.returncode == 0 else (
                    result.stderr[-3000:].decode("utf-8", "replace").strip() or "The operation failed.")
            except (OSError, ValueError) as exc:
                error = str(exc)
            results.put((error, destination))
        threading.Thread(target=worker, daemon=True).start()
        root.after(150, finish)

    forms = [BackupForm(notebook, True, start), BackupForm(notebook, False, start)]
    for form, title in zip(forms, ["Create backup", "Restore backup"]):
        notebook.add(form, text=title)
    progress = ttk.Progressbar(outer, mode="indeterminate")
    progress.pack(fill="x", pady=(12, 4))
    ttk.Label(outer, textvariable=status, wraplength=580).pack(anchor="w")
    def close():
        if busy:
            messagebox.showinfo("Operation in progress", "Wait for the current operation to finish before closing.")
        else:
            root.destroy()
    root.protocol("WM_DELETE_WINDOW", close)
    root.mainloop()


if __name__ == "__main__":
    main(sys.argv[1])
