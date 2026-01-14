.class public final Lcom/fcm2up/Fcm2UpReceiver;
.super Landroid/content/BroadcastReceiver;
.source "Fcm2UpReceiver.kt"


# annotations
.annotation system Ldalvik/annotation/MemberClasses;
    value = {
        Lcom/fcm2up/Fcm2UpReceiver$Companion;
    }
.end annotation

.annotation runtime Lkotlin/Metadata;
    d1 = {
        "\u0000 \n\u0002\u0018\u0002\n\u0002\u0018\u0002\n\u0002\u0008\u0002\n\u0002\u0010\u0002\n\u0000\n\u0002\u0018\u0002\n\u0000\n\u0002\u0018\u0002\n\u0002\u0008\u0006\u0018\u0000 \r2\u00020\u0001:\u0001\rB\u0005\u00a2\u0006\u0002\u0010\u0002J\u0018\u0010\u0003\u001a\u00020\u00042\u0006\u0010\u0005\u001a\u00020\u00062\u0006\u0010\u0007\u001a\u00020\u0008H\u0002J\u0018\u0010\t\u001a\u00020\u00042\u0006\u0010\u0005\u001a\u00020\u00062\u0006\u0010\u0007\u001a\u00020\u0008H\u0002J\u0018\u0010\n\u001a\u00020\u00042\u0006\u0010\u0005\u001a\u00020\u00062\u0006\u0010\u0007\u001a\u00020\u0008H\u0002J\u0010\u0010\u000b\u001a\u00020\u00042\u0006\u0010\u0005\u001a\u00020\u0006H\u0002J\u0018\u0010\u000c\u001a\u00020\u00042\u0006\u0010\u0005\u001a\u00020\u00062\u0006\u0010\u0007\u001a\u00020\u0008H\u0016\u00a8\u0006\u000e"
    }
    d2 = {
        "Lcom/fcm2up/Fcm2UpReceiver;",
        "Landroid/content/BroadcastReceiver;",
        "()V",
        "handleMessage",
        "",
        "context",
        "Landroid/content/Context;",
        "intent",
        "Landroid/content/Intent;",
        "handleNewEndpoint",
        "handleRegistrationFailed",
        "handleUnregistered",
        "onReceive",
        "Companion",
        "fcm2up-shim_debug"
    }
    k = 0x1
    mv = {
        0x1,
        0x9,
        0x0
    }
    xi = 0x30
.end annotation


# static fields
.field private static final ACTION_MESSAGE:Ljava/lang/String; = "org.unifiedpush.android.connector.MESSAGE"

.field private static final ACTION_NEW_ENDPOINT:Ljava/lang/String; = "org.unifiedpush.android.connector.NEW_ENDPOINT"

.field private static final ACTION_REGISTRATION_FAILED:Ljava/lang/String; = "org.unifiedpush.android.connector.REGISTRATION_FAILED"

.field private static final ACTION_UNREGISTERED:Ljava/lang/String; = "org.unifiedpush.android.connector.UNREGISTERED"

.field public static final Companion:Lcom/fcm2up/Fcm2UpReceiver$Companion;

.field private static final EXTRA_BYTES_MESSAGE:Ljava/lang/String; = "bytesMessage"

.field private static final EXTRA_ENDPOINT:Ljava/lang/String; = "endpoint"

.field private static final EXTRA_MESSAGE:Ljava/lang/String; = "message"

.field private static final TAG:Ljava/lang/String; = "FCM2UP"


# direct methods
.method static constructor <clinit>()V
    .registers 2

    new-instance v0, Lcom/fcm2up/Fcm2UpReceiver$Companion;

    const/4 v1, 0x0

    invoke-direct {v0, v1}, Lcom/fcm2up/Fcm2UpReceiver$Companion;-><init>(Lkotlin/jvm/internal/DefaultConstructorMarker;)V

    sput-object v0, Lcom/fcm2up/Fcm2UpReceiver;->Companion:Lcom/fcm2up/Fcm2UpReceiver$Companion;

    return-void
.end method

.method public constructor <init>()V
    .registers 1

    .line 21
    invoke-direct {p0}, Landroid/content/BroadcastReceiver;-><init>()V

    return-void
.end method

.method private final handleMessage(Landroid/content/Context;Landroid/content/Intent;)V
    .registers 7
    .param p1, "context"    # Landroid/content/Context;
    .param p2, "intent"    # Landroid/content/Intent;

    .line 53
    const-string v0, "bytesMessage"

    invoke-virtual {p2, v0}, Landroid/content/Intent;->getByteArrayExtra(Ljava/lang/String;)[B

    move-result-object v0

    .line 54
    .local v0, "bytes":[B
    if-eqz v0, :cond_c

    .line 55
    invoke-static {p1, v0}, Lcom/fcm2up/Fcm2UpShim;->onMessage(Landroid/content/Context;[B)V

    .line 56
    return-void

    .line 60
    :cond_c
    const-string v1, "message"

    invoke-virtual {p2, v1}, Landroid/content/Intent;->getStringExtra(Ljava/lang/String;)Ljava/lang/String;

    move-result-object v1

    .line 61
    .local v1, "message":Ljava/lang/String;
    if-eqz v1, :cond_23

    .line 62
    sget-object v2, Lkotlin/text/Charsets;->UTF_8:Ljava/nio/charset/Charset;

    invoke-virtual {v1, v2}, Ljava/lang/String;->getBytes(Ljava/nio/charset/Charset;)[B

    move-result-object v2

    const-string v3, "getBytes(...)"

    invoke-static {v2, v3}, Lkotlin/jvm/internal/Intrinsics;->checkNotNullExpressionValue(Ljava/lang/Object;Ljava/lang/String;)V

    invoke-static {p1, v2}, Lcom/fcm2up/Fcm2UpShim;->onMessage(Landroid/content/Context;[B)V

    .line 63
    return-void

    .line 66
    :cond_23
    const-string v2, "FCM2UP"

    const-string v3, "MESSAGE intent without message data"

    invoke-static {v2, v3}, Landroid/util/Log;->w(Ljava/lang/String;Ljava/lang/String;)I

    .line 67
    return-void
.end method

.method private final handleNewEndpoint(Landroid/content/Context;Landroid/content/Intent;)V
    .registers 6
    .param p1, "context"    # Landroid/content/Context;
    .param p2, "intent"    # Landroid/content/Intent;

    .line 70
    const-string v0, "endpoint"

    invoke-virtual {p2, v0}, Landroid/content/Intent;->getStringExtra(Ljava/lang/String;)Ljava/lang/String;

    move-result-object v0

    .line 71
    .local v0, "endpoint":Ljava/lang/String;
    if-eqz v0, :cond_c

    .line 72
    invoke-static {p1, v0}, Lcom/fcm2up/Fcm2UpShim;->onNewEndpoint(Landroid/content/Context;Ljava/lang/String;)V

    goto :goto_13

    .line 74
    :cond_c
    const-string v1, "FCM2UP"

    const-string v2, "NEW_ENDPOINT intent without endpoint"

    invoke-static {v1, v2}, Landroid/util/Log;->w(Ljava/lang/String;Ljava/lang/String;)I

    .line 76
    :goto_13
    return-void
.end method

.method private final handleRegistrationFailed(Landroid/content/Context;Landroid/content/Intent;)V
    .registers 4
    .param p1, "context"    # Landroid/content/Context;
    .param p2, "intent"    # Landroid/content/Intent;

    .line 79
    const-string v0, "message"

    invoke-virtual {p2, v0}, Landroid/content/Intent;->getStringExtra(Ljava/lang/String;)Ljava/lang/String;

    move-result-object v0

    .line 80
    .local v0, "reason":Ljava/lang/String;
    invoke-static {p1, v0}, Lcom/fcm2up/Fcm2UpShim;->onRegistrationFailed(Landroid/content/Context;Ljava/lang/String;)V

    .line 81
    return-void
.end method

.method private final handleUnregistered(Landroid/content/Context;)V
    .registers 2
    .param p1, "context"    # Landroid/content/Context;

    .line 84
    invoke-static {p1}, Lcom/fcm2up/Fcm2UpShim;->onUnregistered(Landroid/content/Context;)V

    .line 85
    return-void
.end method


# virtual methods
.method public onReceive(Landroid/content/Context;Landroid/content/Intent;)V
    .registers 6
    .param p1, "context"    # Landroid/content/Context;
    .param p2, "intent"    # Landroid/content/Intent;

    .line 39
    invoke-virtual {p2}, Landroid/content/Intent;->getAction()Ljava/lang/String;

    move-result-object v0

    if-nez v0, :cond_7

    return-void

    .line 41
    .local v0, "action":Ljava/lang/String;
    :cond_7
    new-instance v1, Ljava/lang/StringBuilder;

    invoke-direct {v1}, Ljava/lang/StringBuilder;-><init>()V

    const-string v2, "Received action: "

    invoke-virtual {v1, v2}, Ljava/lang/StringBuilder;->append(Ljava/lang/String;)Ljava/lang/StringBuilder;

    move-result-object v1

    invoke-virtual {v1, v0}, Ljava/lang/StringBuilder;->append(Ljava/lang/String;)Ljava/lang/StringBuilder;

    move-result-object v1

    invoke-virtual {v1}, Ljava/lang/StringBuilder;->toString()Ljava/lang/String;

    move-result-object v1

    const-string v2, "FCM2UP"

    invoke-static {v2, v1}, Landroid/util/Log;->d(Ljava/lang/String;Ljava/lang/String;)I

    .line 43
    invoke-virtual {v0}, Ljava/lang/String;->hashCode()I

    move-result v1

    sparse-switch v1, :sswitch_data_5c

    :goto_26
    goto :goto_5a

    :sswitch_27
    const-string v1, "org.unifiedpush.android.connector.NEW_ENDPOINT"

    invoke-virtual {v0, v1}, Ljava/lang/String;->equals(Ljava/lang/Object;)Z

    move-result v1

    if-nez v1, :cond_30

    goto :goto_26

    .line 45
    :cond_30
    invoke-direct {p0, p1, p2}, Lcom/fcm2up/Fcm2UpReceiver;->handleNewEndpoint(Landroid/content/Context;Landroid/content/Intent;)V

    goto :goto_5a

    .line 43
    :sswitch_34
    const-string v1, "org.unifiedpush.android.connector.REGISTRATION_FAILED"

    invoke-virtual {v0, v1}, Ljava/lang/String;->equals(Ljava/lang/Object;)Z

    move-result v1

    if-nez v1, :cond_3d

    goto :goto_26

    .line 46
    :cond_3d
    invoke-direct {p0, p1, p2}, Lcom/fcm2up/Fcm2UpReceiver;->handleRegistrationFailed(Landroid/content/Context;Landroid/content/Intent;)V

    goto :goto_5a

    .line 43
    :sswitch_41
    const-string v1, "org.unifiedpush.android.connector.UNREGISTERED"

    invoke-virtual {v0, v1}, Ljava/lang/String;->equals(Ljava/lang/Object;)Z

    move-result v1

    if-nez v1, :cond_4a

    goto :goto_26

    .line 47
    :cond_4a
    invoke-direct {p0, p1}, Lcom/fcm2up/Fcm2UpReceiver;->handleUnregistered(Landroid/content/Context;)V

    goto :goto_5a

    .line 43
    :sswitch_4e
    const-string v1, "org.unifiedpush.android.connector.MESSAGE"

    invoke-virtual {v0, v1}, Ljava/lang/String;->equals(Ljava/lang/Object;)Z

    move-result v1

    if-nez v1, :cond_57

    goto :goto_26

    .line 44
    :cond_57
    invoke-direct {p0, p1, p2}, Lcom/fcm2up/Fcm2UpReceiver;->handleMessage(Landroid/content/Context;Landroid/content/Intent;)V

    .line 49
    :goto_5a
    return-void

    nop

    :sswitch_data_5c
    .sparse-switch
        -0x52c601c5 -> :sswitch_4e
        -0x1eb5b3f9 -> :sswitch_41
        -0x18334929 -> :sswitch_34
        0x62b723a0 -> :sswitch_27
    .end sparse-switch
.end method
