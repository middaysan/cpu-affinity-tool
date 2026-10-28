// Test-owned desktop clients; product strings exercise recognition, not publisher trust.
using System;
using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Threading;
using System.Windows.Forms;

#if DISCORD || DISCORD_STUB
[assembly: AssemblyProduct("Discord")]
[assembly: AssemblyCompany("Discord Inc.")]
#elif UPDATER
[assembly: AssemblyProduct("Squirrel fixture")]
[assembly: AssemblyCompany("Test fixture")]
#else
[assembly: AssemblyProduct("Steam")]
[assembly: AssemblyCompany("Valve Corporation")]
#endif
[assembly: AssemblyVersion("1.0.0.0")]
[assembly: AssemblyFileVersion("1.0.0.0")]

static class OwnedClient {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindow(string kind, string title);
    [DllImport("user32.dll")] static extern bool ShowWindowAsync(IntPtr window, int state);
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr window);
    static string WindowTitle(string name) {
        return "CPU Affinity Tool - owned " + name + " fixture - " + Assembly.GetExecutingAssembly().Location;
    }
    static readonly byte[] Resident = new byte[16 * 1024 * 1024];
    static volatile bool Done;
    static void Log(string filename, string text) {
        try { File.AppendAllText(Path.Combine(AppDomain.CurrentDomain.BaseDirectory, filename), text + Environment.NewLine); }
        catch { /* Ordinary clients need no write access to their installation. */ }
    }
    static void Work() {
        for (int i = 0; i < Resident.Length; i += 4096) Resident[i] = 17;
        var total = Stopwatch.StartNew();
        long value = 1;
        while (!Done && total.Elapsed.TotalSeconds < 45) {
            var pulse = Stopwatch.StartNew();
            while (pulse.ElapsedMilliseconds < 8) value = unchecked(value * 6364136223846793005L + 1);
            Thread.Sleep(72);
        }
        GC.KeepAlive(value);
    }
    static void Start(string path, string arguments) {
        Process.Start(new ProcessStartInfo(path, arguments) {
            UseShellExecute = false, CreateNoWindow = true,
            WorkingDirectory = Path.GetDirectoryName(path)
        });
    }
    [STAThread]
    static void Main(string[] args) {
        try { Run(args); }
        catch (Exception error) {
            Log("fixture-error.txt", error.ToString());
        }
    }
    static void Run(string[] args) {
        string root = AppDomain.CurrentDomain.BaseDirectory;
        Log("fixture-events.txt", "started " + Process.GetCurrentProcess().Id + " " + String.Join(" ", args));
#if UPDATER
#if DISCORD_STUB
        if (args.Length != 0 && (args.Length != 2 || args[0] != "--processStart" || args[1] != "Discord.exe")) return;
#else
        if (args.Length != 2 || args[0] != "--processStart" || args[1] != "Discord.exe") return;
#endif
        Start(Path.Combine(root, "app-1.0.1", "Discord.exe"), "--owned-fixture");
        Thread.Sleep(24000);
#else
        if (Array.IndexOf(args, "--fixture-helper") >= 0) { Work(); return; }
#if DISCORD
        const string Name = "Discord";
#else
        const string Name = "Steam";
#endif
        IntPtr existing = FindWindow(null, WindowTitle(Name));
        if (existing != IntPtr.Zero) { ShowWindowAsync(existing, 9); SetForegroundWindow(existing); return; }
#if DISCORD
        Start(Assembly.GetExecutingAssembly().Location, "--fixture-helper");
#else
        Start(Path.Combine(root, "bin", "cef", "cef.win64", "steamwebhelper.exe"), "--fixture-helper");
#endif
        var work = new Thread(Work) { IsBackground = true }; work.Start();
        using (var window = new Form()) {
            window.Text = WindowTitle(Name);
            window.Width = 390; window.Height = 150;
            window.Controls.Add(new Label { Dock = DockStyle.Fill, Text = "Temporary owned test client\nCPU and private RAM workload\nCloses automatically within 45 seconds", Padding = new Padding(12) });
            var timer = new System.Windows.Forms.Timer { Interval = 40000 };
            timer.Tick += delegate { timer.Stop(); window.Close(); };
            window.Shown += delegate { timer.Start(); };
            window.FormClosed += delegate { Done = true; };
            Application.Run(window);
            timer.Dispose();
        }
#endif
    }
}
